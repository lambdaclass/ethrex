use crate::l2::batch::{BatchNumberRequest, GetBatchByBatchNumberRequest};
use crate::l2::l1_message::GetL1MessageProof;
use crate::utils::{RpcErr, RpcNamespace, resolve_namespace};
use axum::extract::State;
use axum::{Json, Router, http::StatusCode, routing::post};
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::Transaction;
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_p2p::types::Node;
use ethrex_p2p::types::NodeRecord;
use ethrex_rpc::RpcHandler as L1RpcHandler;
use ethrex_rpc::{
    GasTipEstimator, NodeData, RpcRequestWrapper,
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
use tokio::{net::TcpListener, sync::Mutex as TokioMutex};
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
    authrpc_addr: SocketAddr,
    storage: Store,
    blockchain: Arc<Blockchain>,
    jwt_secret: Bytes,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    syncer: SyncManager,
    peer_handler: PeerHandler,
    client_version: String,
    valid_delegation_addresses: Vec<Address>,
    sponsor_pk: SecretKey,
    rollup_store: StoreRollup,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
) -> Result<(), RpcErr> {
    // TODO: Refactor how filters are handled,
    // filters are used by the filters endpoints (eth_newFilter, eth_getFilterChanges, ...etc)
    let active_filters = Arc::new(Mutex::new(HashMap::new()));
    let service_context = RpcApiContext {
        l1_ctx: ethrex_rpc::RpcApiContext {
            storage,
            blockchain,
            active_filters: active_filters.clone(),
            syncer: Arc::new(syncer),
            peer_handler,
            node_data: NodeData {
                jwt_secret,
                local_p2p_node,
                local_node_record,
                client_version,
            },
            gas_tip_estimator: Arc::new(TokioMutex::new(GasTipEstimator::new())),
            log_filter_handler,
        },
        valid_delegation_addresses,
        sponsor_pk,
        rollup_store,
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
        .layer(cors)
        .with_state(service_context.clone());
    let http_listener = TcpListener::bind(http_addr)
        .await
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    let http_server = axum::serve(http_listener, http_router)
        .with_graceful_shutdown(ethrex_rpc::shutdown_signal())
        .into_future();
    info!("Starting HTTP server at {http_addr}");

    info!("Not starting Auth-RPC server. The address passed as argument is {authrpc_addr}");

    let _ =
        tokio::try_join!(http_server).inspect_err(|e| info!("Error shutting down servers: {e:?}"));

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
                    Transaction::EIP4844Transaction(wrapped_blob_tx.tx.clone()).hash()
                );
                return Err(RpcErr::InvalidEthrexL2Message(
                    "EIP-4844 transactions are not supported in the L2".to_string(),
                ));
            }
            SendRawTransactionRequest::call(req, context.l1_ctx)
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
        "ethrex_getMessageProof" => GetL1MessageProof::call(req, context).await,
        "ethrex_batchNumber" => BatchNumberRequest::call(req, context).await,
        "ethrex_getBatchByNumber" => GetBatchByBatchNumberRequest::call(req, context).await,
        unknown_ethrex_l2_method => {
            Err(ethrex_rpc::RpcErr::MethodNotFound(unknown_ethrex_l2_method.to_owned()).into())
        }
    }
}
