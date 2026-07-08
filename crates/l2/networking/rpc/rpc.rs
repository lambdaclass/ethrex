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
use axum::extract::ws::WebSocketUpgrade;
use axum::{Json, Router, http::StatusCode, routing::post};
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::Transaction;
use ethrex_crypto::NativeCrypto;
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_p2p::types::SharedLocalNode;
use ethrex_rpc::RpcHandler as L1RpcHandler;
use ethrex_rpc::RpcNamespace as L1RpcNamespace;
use ethrex_rpc::debug::execution_witness::ExecutionWitnessRequest;
use ethrex_rpc::{
    ClientVersion, GasTipEstimator, NodeData, RpcRequestWrapper, RpcRole, RpcStartupError,
    WebSocketConfig, bind_listener,
    types::transaction::SendRawTransactionRequest,
    utils::{RpcRequest, RpcRequestId},
};
use ethrex_storage::Store;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    future::IntoFuture,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Mutex as TokioMutex};
use tokio_util::sync::CancellationToken;
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
    /// Whether L2-specific `ethrex_*` methods are reachable over HTTP/WS.
    pub ethrex_namespace_allowed: bool,
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

/// Binds the L2 RPC listeners (public HTTP and optionally WebSocket; the L2 node exposes no
/// Auth-RPC endpoint) and returns them ready to serve. A bind failure returns a typed
/// [`RpcStartupError`] so the caller can fail fast before spawning any server task.
#[expect(clippy::too_many_arguments)]
pub async fn bind_api(
    cancel_token: CancellationToken,
    http_addr: SocketAddr,
    ws: Option<WebSocketConfig>,
    authrpc_addr: SocketAddr,
    storage: Store,
    blockchain: Arc<Blockchain>,
    jwt_secret: Bytes,
    shared_local_node: SharedLocalNode,
    syncer: Option<Arc<SyncManager>>,
    peer_handler: Option<PeerHandler>,
    client_version: ClientVersion,
    valid_delegation_addresses: Vec<Address>,
    sponsor_pk: SecretKey,
    rollup_store: StoreRollup,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    l2_gas_limit: u64,
    sponsored_gas_limit: u64,
    allowed_namespaces: HashSet<L1RpcNamespace>,
    ethrex_namespace_allowed: bool,
) -> Result<BoundRpc, RpcStartupError> {
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
                shared_local_node,
                client_version,
                extra_data: Bytes::new(),
            },
            gas_tip_estimator: Arc::new(TokioMutex::new(GasTipEstimator::new())),
            log_filter_handler,
            gas_ceil: l2_gas_limit,
            block_worker_channel,
            ws: ws.clone(),
            allowed_namespaces: Arc::new(allowed_namespaces),
        },
        valid_delegation_addresses,
        sponsor_pk,
        rollup_store,
        sponsored_gas_limit,
        ethrex_namespace_allowed,
    };

    // Periodically clean up the active filters for the filters endpoints.
    let filter_cancel = cancel_token.clone();
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(FILTER_DURATION);
        let filters = active_filters.clone();
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    tracing::debug!("Running filter clean task");
                    ethrex_rpc::clean_outdated_filters(filters.clone(), FILTER_DURATION);
                    tracing::debug!("Filter clean task complete");
                }
                _ = filter_cancel.cancelled() => break,
            }
        }
    });

    // All request headers allowed.
    // All methods allowed.
    // All origins allowed.
    // All headers exposed.
    let cors = CorsLayer::permissive();

    // Serve WebSocket on the HTTP listener when both resolve to the same address (matching
    // the L1 behavior and geth/reth/nethermind); otherwise use a separate listener.
    // Conflicting configurations are validated by the node CLI before this is called; a
    // genuine collision still fails the corresponding bind below with a typed error.
    let merged = ws.as_ref().is_some_and(|w| w.addr == http_addr);

    let root = if merged {
        post(handle_http_request).get(ws_upgrade_handler)
    } else {
        post(handle_http_request)
    };
    let http_router = Router::new()
        .route("/", root)
        .layer(cors.clone())
        .with_state(service_context.clone());

    let http_listener = bind_listener(RpcRole::Http, http_addr).await?;
    if merged {
        info!("HTTP-RPC + WebSocket server listening on {http_addr}");
    } else {
        info!("HTTP-RPC server listening on {http_addr}");
    }
    info!("Not starting Auth-RPC server. The address passed as argument is {authrpc_addr}");

    let ws_bound = match &ws {
        Some(ws_config) if !merged => {
            let ws_router = Router::new()
                .route("/", axum::routing::any(ws_upgrade_handler))
                .layer(cors)
                .with_state(service_context);
            let ws_listener = bind_listener(RpcRole::Ws, ws_config.addr).await?;
            info!("WebSocket server listening on {}", ws_config.addr);
            Some((ws_listener, ws_router))
        }
        _ => None,
    };

    Ok(BoundRpc {
        http: (http_listener, http_router),
        ws: ws_bound,
        cancel_token,
    })
}

/// L2 RPC listeners bound and ready to serve (HTTP + optional WebSocket; the L2 node has no
/// Auth-RPC endpoint). Split from serving so a bind failure fails fast in the foreground.
pub struct BoundRpc {
    http: (TcpListener, Router),
    ws: Option<(TcpListener, Router)>,
    cancel_token: CancellationToken,
}

impl BoundRpc {
    /// Serves the bound L2 listeners until graceful shutdown, returning the first serve
    /// error so the caller can treat a runtime serve failure as fatal.
    pub async fn serve(self) -> Result<(), RpcErr> {
        let cancel_token = self.cancel_token;
        let (http_listener, http_router) = self.http;
        let http_server = axum::serve(http_listener, http_router)
            .with_graceful_shutdown(ethrex_rpc::shutdown_signal(cancel_token.clone()))
            .into_future();

        if let Some((ws_listener, ws_router)) = self.ws {
            let ws_server = axum::serve(ws_listener, ws_router)
                .with_graceful_shutdown(ethrex_rpc::shutdown_signal(cancel_token.clone()))
                .into_future();
            tokio::try_join!(http_server, ws_server)
                .map_err(|e| RpcErr::Internal(e.to_string()))?;
        } else {
            tokio::try_join!(http_server).map_err(|e| RpcErr::Internal(e.to_string()))?;
        }
        Ok(())
    }
}

/// Binds and serves the L2 RPC API. Compatibility wrapper over [`bind_api`] +
/// [`BoundRpc::serve`]; the node uses `bind_api` directly so a bind failure fails fast.
#[expect(clippy::too_many_arguments)]
pub async fn start_api(
    http_addr: SocketAddr,
    ws: Option<WebSocketConfig>,
    authrpc_addr: SocketAddr,
    storage: Store,
    blockchain: Arc<Blockchain>,
    jwt_secret: Bytes,
    shared_local_node: SharedLocalNode,
    syncer: Option<Arc<SyncManager>>,
    peer_handler: Option<PeerHandler>,
    client_version: ClientVersion,
    valid_delegation_addresses: Vec<Address>,
    sponsor_pk: SecretKey,
    rollup_store: StoreRollup,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    l2_gas_limit: u64,
    sponsored_gas_limit: u64,
    allowed_namespaces: HashSet<L1RpcNamespace>,
    ethrex_namespace_allowed: bool,
) -> Result<(), RpcErr> {
    bind_api(
        CancellationToken::new(),
        http_addr,
        ws,
        authrpc_addr,
        storage,
        blockchain,
        jwt_secret,
        shared_local_node,
        syncer,
        peer_handler,
        client_version,
        valid_delegation_addresses,
        sponsor_pk,
        rollup_store,
        log_filter_handler,
        l2_gas_limit,
        sponsored_gas_limit,
        allowed_namespaces,
        ethrex_namespace_allowed,
    )
    .await
    .map_err(|e| RpcErr::Internal(e.to_string()))?
    .serve()
    .await
}

/// WebSocket upgrade handler shared by the merged HTTP listener and the standalone WS
/// listener. A `GET` without a valid upgrade is rejected with a 4xx status (`400` for
/// missing/invalid upgrade headers).
async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    State(ctx): State<RpcApiContext>,
) -> axum::response::Response {
    ws.on_upgrade(|mut socket| async move {
        ethrex_rpc::handle_websocket(&mut socket, &ctx.l1_ctx, |req| {
            let c = ctx.clone();
            async move { map_http_requests(&req, c).await }
        })
        .await;
    })
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
        // `Eth` is handled here (not delegated to L1) so that L2-specific
        // overrides in `map_eth_requests` see the full L2 context. Because we
        // bypass `ethrex_rpc::map_http_requests`, the `--http.api` allowlist
        // guard must be enforced explicitly here. Any future namespace that
        // gains a dedicated arm before the `_` fallthrough must do the same.
        Ok(RpcNamespace::L1RpcNamespace(L1RpcNamespace::Eth)) => {
            if !context
                .l1_ctx
                .allowed_namespaces
                .contains(&L1RpcNamespace::Eth)
            {
                return Err(RpcErr::L1RpcErr(ethrex_rpc::RpcErr::MethodNotFound(
                    req.method.clone(),
                )));
            }
            map_eth_requests(req, context).await
        }
        Ok(RpcNamespace::EthrexL2) => {
            if !context.ethrex_namespace_allowed {
                return Err(RpcErr::L1RpcErr(ethrex_rpc::RpcErr::MethodNotFound(
                    req.method.clone(),
                )));
            }
            map_l2_requests(req, context).await
        }
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
                    Transaction::EIP4844Transaction(wrapped_blob_tx.tx).hash(&NativeCrypto)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_storage::{EngineType, Store};
    use ethrex_storage_rollup::EngineTypeRollup;

    async fn test_context(ethrex_namespace_allowed: bool) -> RpcApiContext {
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let l1_ctx = ethrex_rpc::test_utils::default_context_with_storage(storage).await;
        let rollup_store = ethrex_storage_rollup::StoreRollup::new(
            std::path::Path::new(""),
            EngineTypeRollup::InMemory,
        )
        .expect("Failed to create rollup store");
        RpcApiContext {
            l1_ctx,
            valid_delegation_addresses: vec![],
            sponsor_pk: SecretKey::from_byte_array(&[0xab; 32]).unwrap(),
            rollup_store,
            sponsored_gas_limit: 0,
            ethrex_namespace_allowed,
        }
    }

    /// With `--http.api.ethrex=false`, L2-specific `ethrex_*` methods must be
    /// rejected at the dispatcher with MethodNotFound and never reach handlers.
    #[tokio::test]
    async fn ethrex_namespace_blocked_when_disabled() {
        let body = r#"{"jsonrpc":"2.0","method":"ethrex_batchNumber","params":[],"id":1}"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();
        let context = test_context(false).await;

        let result = map_http_requests(&request, context).await;
        match result {
            Err(RpcErr::L1RpcErr(ethrex_rpc::RpcErr::MethodNotFound(method))) => {
                assert_eq!(method, "ethrex_batchNumber");
            }
            other => panic!("expected MethodNotFound, got {other:?}"),
        }
    }
}
