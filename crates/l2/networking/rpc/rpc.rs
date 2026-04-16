use crate::l2::batch::{
    BatchNumberRequest, GetBatchByBatchBlockNumberRequest, GetBatchByBatchNumberRequest,
};
use crate::l2::execution_witness::handle_execution_witness;
use crate::l2::fees::{
    GetBaseFeeVaultAddress, GetL1BlobBaseFeeRequest, GetL1FeeVaultAddress, GetOperatorFee,
    GetOperatorFeeVaultAddress,
};
use crate::l2::messages::GetL1MessageProof;
use crate::utils::{RpcErr, RpcNamespace, resolve_namespace};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::{Json, Router, http::StatusCode, routing::post};
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::Transaction;
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_p2p::types::Node;
use ethrex_p2p::types::NodeRecord;
use ethrex_rpc::RpcHandler as L1RpcHandler;
use ethrex_rpc::debug::execution_witness::ExecutionWitnessRequest;
use ethrex_rpc::{
    ClientVersion, GasTipEstimator, NodeData, RpcRequestWrapper,
    types::transaction::SendRawTransactionRequest,
    utils::{RpcRequest, RpcRequestId},
};
use ethrex_storage::Store;
use serde_json::Value;
use std::{
    collections::HashMap,
    future::IntoFuture,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Mutex as TokioMutex, broadcast},
};
use tower_http::cors::CorsLayer;
use tracing::{debug, info};
use tracing_subscriber::{EnvFilter, Registry, reload};

use crate::l2::transaction::SponsoredTx;
use ethrex_common::Address;
use ethrex_storage_rollup::StoreRollup;
use secp256k1::SecretKey;

#[derive(Debug, Clone)]
pub struct RpcApiContext {
    pub l1_ctx: ethrex_rpc::RpcApiContext,
    pub valid_delegation_addresses: Vec<Address>,
    pub sponsor_pk: SecretKey,
    pub rollup_store: StoreRollup,
    pub sponsored_gas_limit: u64,
}

pub trait RpcHandler: Sized {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr>;

    async fn call(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        request.handle(context).await
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr>;
}

pub const FILTER_DURATION: Duration = {
    if cfg!(test) {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5 * 60)
    }
};

#[expect(clippy::too_many_arguments)]
pub async fn start_api(
    http_addr: SocketAddr,
    ws_addr: Option<SocketAddr>,
    authrpc_addr: SocketAddr,
    storage: Store,
    blockchain: Arc<Blockchain>,
    jwt_secret: Bytes,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    syncer: Option<Arc<SyncManager>>,
    peer_handler: Option<PeerHandler>,
    client_version: ClientVersion,
    valid_delegation_addresses: Vec<Address>,
    sponsor_pk: SecretKey,
    rollup_store: StoreRollup,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    l2_gas_limit: u64,
    sponsored_gas_limit: u64,
    // Broadcast sender for new block header notifications. When `ws_addr` is
    // `Some`, callers should create a `broadcast::channel` and pass the sender
    // here. The same sender clone should be given to the block producer so it
    // can publish headers after sealing each block.
    new_heads_sender: Option<broadcast::Sender<Value>>,
) -> Result<(), RpcErr> {
    // TODO: Refactor how filters are handled,
    // filters are used by the filters endpoints (eth_newFilter, eth_getFilterChanges, ...etc)
    if sponsored_gas_limit == 0 {
        tracing::warn!(
            "sponsored_gas_limit is set to 0; all sponsored transactions will be rejected"
        );
    }

    let active_filters = Arc::new(Mutex::new(HashMap::new()));
    let block_worker_channel = ethrex_rpc::start_block_executor(blockchain.clone());
    let service_context = RpcApiContext {
        l1_ctx: ethrex_rpc::RpcApiContext {
            storage,
            blockchain,
            active_filters: active_filters.clone(),
            syncer,
            peer_handler,
            node_data: NodeData {
                jwt_secret,
                local_p2p_node,
                local_node_record,
                client_version,
                extra_data: Bytes::new(),
            },
            gas_tip_estimator: Arc::new(TokioMutex::new(GasTipEstimator::new())),
            log_filter_handler,
            gas_ceil: l2_gas_limit,
            block_worker_channel,
            new_heads_sender,
        },
        valid_delegation_addresses,
        sponsor_pk,
        rollup_store,
        sponsored_gas_limit,
    };

    // Periodically clean up the active filters for the filters endpoints.
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(FILTER_DURATION);
        let filters = active_filters.clone();
        loop {
            interval.tick().await;
            tracing::info!("Running filter clean task");
            ethrex_rpc::clean_outdated_filters(filters.clone(), FILTER_DURATION);
            tracing::info!("Filter clean task complete");
        }
    });

    // All request headers allowed.
    // All methods allowed.
    // All origins allowed.
    // All headers exposed.
    let cors = CorsLayer::permissive();

    let http_router = Router::new()
        .route("/", post(handle_http_request))
        .layer(cors.clone())
        .with_state(service_context.clone());
    let http_listener = TcpListener::bind(http_addr)
        .await
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    let http_server = axum::serve(http_listener, http_router)
        .with_graceful_shutdown(ethrex_rpc::shutdown_signal())
        .into_future();
    info!("Starting HTTP server at {http_addr}");

    info!("Not starting Auth-RPC server. The address passed as argument is {authrpc_addr}");

    if let Some(address) = ws_addr {
        let ws_handler = |ws: WebSocketUpgrade, ctx: State<RpcApiContext>| async move {
            ws.on_upgrade(|socket| handle_websocket(socket, ctx.0))
        };
        let ws_router = Router::new()
            .route("/", axum::routing::any(ws_handler))
            .layer(cors)
            .with_state(service_context);
        let ws_listener = TcpListener::bind(address)
            .await
            .map_err(|error| RpcErr::Internal(error.to_string()))?;
        let ws_server = axum::serve(ws_listener, ws_router)
            .with_graceful_shutdown(ethrex_rpc::shutdown_signal())
            .into_future();
        info!("Starting L2 WS server at {address}");

        let _ = tokio::try_join!(http_server, ws_server)
            .inspect_err(|e| info!("Error shutting down servers: {e:?}"));
    } else {
        let _ = tokio::try_join!(http_server)
            .inspect_err(|e| info!("Error shutting down servers: {e:?}"));
    }

    Ok(())
}

async fn handle_http_request(
    State(service_context): State<RpcApiContext>,
    body: String,
) -> Result<Json<Value>, StatusCode> {
    let res = match serde_json::from_str::<RpcRequestWrapper>(&body) {
        Ok(RpcRequestWrapper::Single(request)) => {
            let res = map_http_requests(&request, service_context).await;
            ethrex_rpc::rpc_response(request.id, res).map_err(|_| StatusCode::BAD_REQUEST)?
        }
        Ok(RpcRequestWrapper::Multiple(requests)) => {
            let mut responses = Vec::new();
            for req in requests {
                let res = map_http_requests(&req, service_context.clone()).await;
                responses.push(
                    ethrex_rpc::rpc_response(req.id, res).map_err(|_| StatusCode::BAD_REQUEST)?,
                );
            }
            serde_json::to_value(responses).map_err(|_| StatusCode::BAD_REQUEST)?
        }
        Err(_) => ethrex_rpc::rpc_response(
            RpcRequestId::String("".to_string()),
            Err(ethrex_rpc::RpcErr::BadParams(
                "Invalid request body".to_string(),
            )),
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?,
    };
    Ok(Json(res))
}

/// Handle requests that can come from either clients or other users
pub async fn map_http_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match resolve_namespace(&req.method) {
        Ok(RpcNamespace::L1RpcNamespace(ethrex_rpc::RpcNamespace::Eth)) => {
            map_eth_requests(req, context).await
        }
        Ok(RpcNamespace::EthrexL2) => map_l2_requests(req, context).await,
        _ => ethrex_rpc::map_http_requests(req, context.l1_ctx)
            .await
            .map_err(RpcErr::L1RpcErr),
    }
}

pub async fn map_eth_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "eth_sendRawTransaction" => {
            let tx = SendRawTransactionRequest::parse(&req.params)?;
            if let SendRawTransactionRequest::EIP4844(wrapped_blob_tx) = tx {
                debug!(
                    "EIP-4844 transaction are not supported in the L2: {:#x}",
                    Transaction::EIP4844Transaction(wrapped_blob_tx.tx).hash()
                );
                return Err(RpcErr::InvalidEthrexL2Message(
                    "EIP-4844 transactions are not supported in the L2".to_string(),
                ));
            }
            SendRawTransactionRequest::call(req, context.l1_ctx)
                .await
                .map_err(RpcErr::L1RpcErr)
        }
        "debug_executionWitness" => {
            let request = ExecutionWitnessRequest::parse(&req.params)?;
            handle_execution_witness(&request, context)
                .await
                .map_err(RpcErr::L1RpcErr)
        }
        _other_eth_method => ethrex_rpc::map_eth_requests(req, context.l1_ctx)
            .await
            .map_err(RpcErr::L1RpcErr),
    }
}

pub async fn map_l2_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "ethrex_sendTransaction" => SponsoredTx::call(req, context).await,
        "ethrex_getL1MessageProof" => GetL1MessageProof::call(req, context).await,
        "ethrex_batchNumber" => BatchNumberRequest::call(req, context).await,
        "ethrex_getBatchByBlock" => GetBatchByBatchBlockNumberRequest::call(req, context).await,
        "ethrex_getBatchByNumber" => GetBatchByBatchNumberRequest::call(req, context).await,
        "ethrex_getBaseFeeVaultAddress" => GetBaseFeeVaultAddress::call(req, context).await,
        "ethrex_getOperatorFeeVaultAddress" => GetOperatorFeeVaultAddress::call(req, context).await,
        "ethrex_getOperatorFee" => GetOperatorFee::call(req, context).await,
        "ethrex_getL1FeeVaultAddress" => GetL1FeeVaultAddress::call(req, context).await,
        "ethrex_getL1BlobBaseFee" => GetL1BlobBaseFeeRequest::call(req, context).await,
        unknown_ethrex_l2_method => {
            Err(ethrex_rpc::RpcErr::MethodNotFound(unknown_ethrex_l2_method.to_owned()).into())
        }
    }
}

/// Handle a WebSocket connection.
///
/// Supports eth_subscribe / eth_unsubscribe for "newHeads" in addition to
/// regular JSON-RPC request-response calls that work the same as over HTTP.
/// Subscription functionality is provided by ethrex_rpc (L1 crate).
async fn handle_websocket(mut socket: WebSocket, context: RpcApiContext) {
    // subscription_id -> broadcast::Receiver<Value>
    let mut subscriptions: HashMap<String, broadcast::Receiver<Value>> = HashMap::new();
    // Channel for the write loop to receive outbound messages.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    loop {
        tokio::select! {
            // Process incoming WS messages (JSON-RPC requests).
            msg = socket.recv() => {
                let Some(msg) = msg else {
                    // Connection closed.
                    break;
                };
                let body = match msg {
                    Ok(Message::Text(text)) => text.to_string(),
                    Ok(Message::Close(_)) => break,
                    // Ignore ping/pong/binary frames.
                    Ok(_) => continue,
                    Err(_) => break,
                };

                let response = handle_ws_request(&body, &context, &mut subscriptions, &out_tx).await;
                if let Some(resp) = response
                    && socket.send(Message::Text(resp.into())).await.is_err()
                {
                    break;
                }
            }

            // Push subscription notifications for all active subscriptions.
            _ = ethrex_rpc::drain_subscriptions(&mut subscriptions, &out_tx) => {}

            // Send any pending outbound messages (subscription notifications).
            Some(msg) = out_rx.recv() => {
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
        }
    }

    // Connection closed — subscriptions are dropped automatically when the
    // HashMap goes out of scope.
}

/// Process an incoming JSON-RPC request over WebSocket.
/// Returns `Some(response_text)` for request-response calls.
/// For eth_subscribe / eth_unsubscribe the response is also returned inline.
async fn handle_ws_request(
    body: &str,
    context: &RpcApiContext,
    subscriptions: &mut HashMap<String, broadcast::Receiver<Value>>,
    _out_tx: &tokio::sync::mpsc::UnboundedSender<String>,
) -> Option<String> {
    let req: RpcRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            let resp = ethrex_rpc::rpc_response(
                RpcRequestId::String("".to_string()),
                Err::<Value, _>(ethrex_rpc::RpcErr::BadParams(
                    "Invalid request body".to_string(),
                )),
            )
            .ok()?;
            return Some(resp.to_string());
        }
    };

    match req.method.as_str() {
        "eth_subscribe" => {
            // Delegate to L1's implementation, which reads from context.l1_ctx.new_heads_sender.
            let result = ethrex_rpc::handle_eth_subscribe(&req, &context.l1_ctx, subscriptions)
                .map_err(RpcErr::L1RpcErr);
            let resp = ethrex_rpc::rpc_response(req.id, result).ok()?;
            Some(resp.to_string())
        }
        "eth_unsubscribe" => {
            // Delegate to L1's implementation.
            let result =
                ethrex_rpc::handle_eth_unsubscribe(&req, subscriptions).map_err(RpcErr::L1RpcErr);
            let resp = ethrex_rpc::rpc_response(req.id, result).ok()?;
            Some(resp.to_string())
        }
        _ => {
            let res = map_http_requests(&req, context.clone()).await;
            let resp = ethrex_rpc::rpc_response(req.id, res).ok()?;
            Some(resp.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ethrex_rpc::{
        NEW_HEADS_CHANNEL_CAPACITY, broadcast, build_subscription_notification,
        generate_subscription_id, handle_eth_unsubscribe,
    };
    use serde_json::{Value, json};

    use super::*;

    // ── NEW_HEADS_CHANNEL_CAPACITY ───────────────────────────────────────────

    #[test]
    fn channel_capacity_constant_is_sensible() {
        assert!(
            NEW_HEADS_CHANNEL_CAPACITY >= 16,
            "channel capacity should handle at least 16 buffered headers"
        );
    }

    // ── generate_subscription_id ─────────────────────────────────────────────

    #[test]
    fn subscription_id_has_hex_prefix() {
        let id = generate_subscription_id();
        assert!(
            id.starts_with("0x"),
            "subscription ID must start with 0x, got: {id}"
        );
    }

    #[test]
    fn subscription_ids_are_unique() {
        let ids: Vec<String> = (0..10).map(|_| generate_subscription_id()).collect();
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "all generated subscription IDs must be unique"
        );
    }

    #[test]
    fn subscription_id_is_16_hex_chars_after_prefix() {
        let id = generate_subscription_id();
        let hex_part = id.strip_prefix("0x").expect("must start with 0x");
        assert_eq!(
            hex_part.len(),
            16,
            "subscription ID hex part should be 16 chars (u64), got: {hex_part}"
        );
    }

    // ── build_subscription_notification ──────────────────────────────────────

    #[test]
    fn notification_has_correct_jsonrpc_method_and_params() {
        let sub_id = "0x0000000000000001";
        let header = json!({"number": "0x1", "hash": "0xabc"});

        let notification_str = build_subscription_notification(sub_id, header.clone());
        let notification: Value =
            serde_json::from_str(&notification_str).expect("must be valid JSON");

        assert_eq!(notification["jsonrpc"], "2.0");
        assert_eq!(notification["method"], "eth_subscription");
        assert_eq!(notification["params"]["subscription"], sub_id);
        assert_eq!(notification["params"]["result"], header);
    }

    // ── Broadcast channel send/receive ───────────────────────────────────────

    #[test]
    fn broadcast_channel_delivers_header_to_subscriber() {
        let (tx, mut rx) = broadcast::channel::<Value>(NEW_HEADS_CHANNEL_CAPACITY);

        let header = json!({"number": "0x42", "hash": "0xdeadbeef"});
        tx.send(header.clone()).expect("send must succeed");

        let received = rx.try_recv().expect("receiver must have a message");
        assert_eq!(received, header);
    }

    #[test]
    fn broadcast_channel_delivers_to_multiple_subscribers() {
        let (tx, mut rx1) = broadcast::channel::<Value>(NEW_HEADS_CHANNEL_CAPACITY);
        let mut rx2 = tx.subscribe();

        let header = json!({"number": "0x1"});
        tx.send(header.clone()).expect("send must succeed");

        assert_eq!(rx1.try_recv().expect("rx1 must receive"), header);
        assert_eq!(rx2.try_recv().expect("rx2 must receive"), header);
    }

    #[test]
    fn broadcast_channel_empty_when_no_messages_sent() {
        let (_tx, mut rx) = broadcast::channel::<Value>(NEW_HEADS_CHANNEL_CAPACITY);
        assert!(
            rx.try_recv().is_err(),
            "channel should be empty before any send"
        );
    }

    // ── handle_eth_unsubscribe ───────────────────────────────────────────────

    #[test]
    fn unsubscribe_returns_true_when_subscription_exists() {
        let (tx, rx) = broadcast::channel::<Value>(8);
        drop(tx); // sender not needed for this test
        let sub_id = "0x0000000000000001".to_string();
        let mut subscriptions: HashMap<String, broadcast::Receiver<Value>> = HashMap::new();
        subscriptions.insert(sub_id.clone(), rx);

        let req = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_unsubscribe".to_string(),
            params: Some(vec![json!(sub_id)]),
        };

        let result = handle_eth_unsubscribe(&req, &mut subscriptions);
        assert_eq!(result.expect("must succeed"), Value::Bool(true));
        assert!(subscriptions.is_empty(), "subscription must be removed");
    }

    #[test]
    fn unsubscribe_returns_false_when_subscription_does_not_exist() {
        let mut subscriptions: HashMap<String, broadcast::Receiver<Value>> = HashMap::new();

        let req = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_unsubscribe".to_string(),
            params: Some(vec![json!("0x0000000000000099")]),
        };

        let result = handle_eth_unsubscribe(&req, &mut subscriptions);
        assert_eq!(result.expect("must succeed"), Value::Bool(false));
    }

    #[test]
    fn unsubscribe_errors_when_no_params() {
        let mut subscriptions: HashMap<String, broadcast::Receiver<Value>> = HashMap::new();

        let req = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_unsubscribe".to_string(),
            params: None,
        };

        let result = handle_eth_unsubscribe(&req, &mut subscriptions);
        assert!(result.is_err(), "must return error when params are missing");
    }
}
