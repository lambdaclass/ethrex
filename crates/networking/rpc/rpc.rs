use crate::authentication::authenticate;
use crate::debug::execution_witness::ExecutionWitnessRequest;
use crate::engine::{
    ExchangeCapabilitiesRequest,
    blobs::BlobsV1Request,
    exchange_transition_config::ExchangeTransitionConfigV1Req,
    fork_choice::{ForkChoiceUpdatedV1, ForkChoiceUpdatedV2, ForkChoiceUpdatedV3},
    payload::{
        GetPayloadBodiesByHashV1Request, GetPayloadBodiesByRangeV1Request, GetPayloadV1Request,
        GetPayloadV2Request, GetPayloadV3Request, GetPayloadV4Request, NewPayloadV1Request,
        NewPayloadV2Request, NewPayloadV3Request, NewPayloadV4Request,
    },
};
use crate::eth::client::Config;
use crate::eth::{
    account::{
        GetBalanceRequest, GetCodeRequest, GetProofRequest, GetStorageAtRequest,
        GetTransactionCountRequest,
    },
    block::{
        BlockNumberRequest, GetBlobBaseFee, GetBlockByHashRequest, GetBlockByNumberRequest,
        GetBlockReceiptsRequest, GetBlockTransactionCountRequest, GetRawBlockRequest,
        GetRawHeaderRequest, GetRawReceipts,
    },
    client::{ChainId, Syncing},
    fee_market::FeeHistoryRequest,
    filter::{self, ActiveFilters, DeleteFilterRequest, FilterChangesRequest, NewFilterRequest},
    gas_price::GasPrice,
    gas_tip_estimator::GasTipEstimator,
    logs::LogsFilter,
    transaction::{
        CallRequest, CreateAccessListRequest, EstimateGasRequest, GetRawTransaction,
        GetTransactionByBlockHashAndIndexRequest, GetTransactionByBlockNumberAndIndexRequest,
        GetTransactionByHashRequest, GetTransactionReceiptRequest,
    },
};
use crate::tracing::{TraceBlockByNumberRequest, TraceTransactionRequest};
use crate::types::transaction::SendRawTransactionRequest;
use crate::utils::{
    RpcErr, RpcErrorMetadata, RpcErrorResponse, RpcNamespace, RpcRequest, RpcRequestId,
    RpcSuccessResponse,
};
use crate::{admin, net};
use crate::{eth, mempool};
use axum::extract::{DefaultBodyLimit, State};
use axum::{Json, Router, http::StatusCode, routing::post};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::DEFAULT_BUILDER_GAS_CEIL;
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_p2p::types::Node;
use ethrex_p2p::types::NodeRecord;
use ethrex_storage::Store;
use serde::Deserialize;
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
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, Registry, reload};

#[cfg(all(feature = "jemalloc_profiling", target_os = "linux"))]
use axum::response::IntoResponse;
// only works on linux
#[cfg(all(feature = "jemalloc_profiling", target_os = "linux"))]
pub async fn handle_get_heap() -> Result<impl IntoResponse, (StatusCode, String)> {
    let Some(mutex) = jemalloc_pprof::PROF_CTL.as_ref() else {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "jemalloc profiling is not available".into(),
        ));
    };
    let mut prof_ctl = mutex.lock().await;
    require_profiling_activated(&prof_ctl)?;
    let pprof = prof_ctl
        .dump_pprof()
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(pprof)
}

/// Checks whether jemalloc profiling is activated an returns an error response if not.
#[cfg(all(feature = "jemalloc_profiling", target_os = "linux"))]
fn require_profiling_activated(
    prof_ctl: &jemalloc_pprof::JemallocProfCtl,
) -> Result<(), (StatusCode, String)> {
    if prof_ctl.activated() {
        Ok(())
    } else {
        Err((
            axum::http::StatusCode::FORBIDDEN,
            "heap profiling not activated".into(),
        ))
    }
}

#[cfg(all(feature = "jemalloc_profiling", target_os = "linux"))]
pub async fn handle_get_heap_flamegraph() -> Result<impl IntoResponse, (StatusCode, String)> {
    use axum::body::Body;
    use axum::http::header::CONTENT_TYPE;
    use axum::response::Response;

    let Some(mutex) = jemalloc_pprof::PROF_CTL.as_ref() else {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "jemalloc profiling is not available".into(),
        ));
    };
    let mut prof_ctl = mutex.lock().await;
    require_profiling_activated(&prof_ctl)?;
    let svg = prof_ctl
        .dump_flamegraph()
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Response::builder()
        .header(CONTENT_TYPE, "image/svg+xml")
        .body(Body::from(svg))
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

// Feature-disabled stubs (no dependency on jemalloc_pprof)
#[cfg(not(all(feature = "jemalloc_profiling", target_os = "linux")))]
pub async fn handle_get_heap() -> Result<(), (StatusCode, String)> {
    Err((
        StatusCode::NOT_IMPLEMENTED,
        "jemalloc profiling is not available (build with `ethrex-rpc/jemalloc_profiling`, it only works on linux)".into(),
    ))
}

#[cfg(not(all(feature = "jemalloc_profiling", target_os = "linux")))]
pub async fn handle_get_heap_flamegraph() -> Result<(), (StatusCode, String)> {
    Err((
        StatusCode::NOT_IMPLEMENTED,
        "jemalloc profiling is not available (build with `ethrex-rpc/jemalloc_profiling`, it only works on linux)".into(),
    ))
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum RpcRequestWrapper {
    Single(RpcRequest),
    Multiple(Vec<RpcRequest>),
}

#[derive(Debug, Clone)]
pub struct RpcApiContext {
    pub storage: Store,
    pub blockchain: Arc<Blockchain>,
    pub active_filters: ActiveFilters,
    pub syncer: Arc<SyncManager>,
    pub peer_handler: PeerHandler,
    pub node_data: NodeData,
    pub gas_tip_estimator: Arc<TokioMutex<GasTipEstimator>>,
    pub log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    pub gas_ceil: u64,
}

#[derive(Debug, Clone)]
pub struct NodeData {
    pub jwt_secret: Bytes,
    pub local_p2p_node: Node,
    pub local_node_record: NodeRecord,
    pub client_version: String,
}

#[allow(async_fn_in_trait)]
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

#[allow(clippy::too_many_arguments)]
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
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    gas_ceil: Option<u64>,
) -> Result<(), RpcErr> {
    // TODO: Refactor how filters are handled,
    // filters are used by the filters endpoints (eth_newFilter, eth_getFilterChanges, ...etc)
    let active_filters = Arc::new(Mutex::new(HashMap::new()));
    let service_context = RpcApiContext {
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
        gas_ceil: gas_ceil.unwrap_or(DEFAULT_BUILDER_GAS_CEIL),
    };

    // Periodically clean up the active filters for the filters endpoints.
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(FILTER_DURATION);
        let filters = active_filters.clone();
        loop {
            interval.tick().await;
            tracing::debug!("Running filter clean task");
            filter::clean_outdated_filters(filters.clone(), FILTER_DURATION);
            tracing::debug!("Filter clean task complete");
        }
    });

    // All request headers allowed.
    // All methods allowed.
    // All origins allowed.
    // All headers exposed.
    let cors = CorsLayer::permissive();

    let http_router = Router::new()
        .route("/debug/pprof/allocs", axum::routing::get(handle_get_heap))
        .route(
            "/debug/pprof/allocs/flamegraph",
            axum::routing::get(handle_get_heap_flamegraph),
        )
        .route("/", post(handle_http_request))
        .layer(cors)
        .with_state(service_context.clone());
    let http_listener = TcpListener::bind(http_addr)
        .await
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    let http_server = axum::serve(http_listener, http_router)
        .with_graceful_shutdown(shutdown_signal())
        .into_future();
    info!("Starting HTTP server at {http_addr}");

    let authrpc_handler = |ctx, auth, body| async { handle_authrpc_request(ctx, auth, body).await };
    let authrpc_router = Router::new()
        .route("/", post(authrpc_handler))
        .with_state(service_context)
        // Bump the body limit for the engine API to 256MB
        // This is needed to receive payloads bigger than the default limit of 2MB
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024));

    let authrpc_listener = TcpListener::bind(authrpc_addr)
        .await
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    let authrpc_server = axum::serve(authrpc_listener, authrpc_router)
        .with_graceful_shutdown(shutdown_signal())
        .into_future();
    info!("Starting Auth-RPC server at {authrpc_addr}");

    let _ = tokio::try_join!(authrpc_server, http_server)
        .inspect_err(|e| error!("Error shutting down servers: {e:?}"));

    Ok(())
}

pub async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
}

async fn handle_http_request(
    State(service_context): State<RpcApiContext>,
    body: String,
) -> Result<Json<Value>, StatusCode> {
    let res = match serde_json::from_str::<RpcRequestWrapper>(&body) {
        Ok(RpcRequestWrapper::Single(request)) => {
            let res = map_http_requests(&request, service_context).await;
            rpc_response(request.id, res).map_err(|_| StatusCode::BAD_REQUEST)?
        }
        Ok(RpcRequestWrapper::Multiple(requests)) => {
            let mut responses = Vec::new();
            for req in requests {
                let res = map_http_requests(&req, service_context.clone()).await;
                responses.push(rpc_response(req.id, res).map_err(|_| StatusCode::BAD_REQUEST)?);
            }
            serde_json::to_value(responses).map_err(|_| StatusCode::BAD_REQUEST)?
        }
        Err(_) => rpc_response(
            RpcRequestId::String("".to_string()),
            Err(RpcErr::BadParams("Invalid request body".to_string())),
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?,
    };
    Ok(Json(res))
}

pub async fn handle_authrpc_request(
    State(service_context): State<RpcApiContext>,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    body: String,
) -> Result<Json<Value>, StatusCode> {
    let req: RpcRequest = match serde_json::from_str(&body) {
        Ok(req) => req,
        Err(_) => {
            return Ok(Json(
                rpc_response(
                    RpcRequestId::String("".to_string()),
                    Err(RpcErr::BadParams("Invalid request body".to_string())),
                )
                .map_err(|_| StatusCode::BAD_REQUEST)?,
            ));
        }
    };
    match authenticate(&service_context.node_data.jwt_secret, auth_header) {
        Err(error) => Ok(Json(
            rpc_response(req.id, Err(error)).map_err(|_| StatusCode::BAD_REQUEST)?,
        )),
        Ok(()) => {
            // Proceed with the request
            let res = map_authrpc_requests(&req, service_context).await;
            Ok(Json(
                rpc_response(req.id, res).map_err(|_| StatusCode::BAD_REQUEST)?,
            ))
        }
    }
}

/// Handle requests that can come from either clients or other users
pub async fn map_http_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.namespace() {
        Ok(RpcNamespace::Eth) => map_eth_requests(req, context).await,
        Ok(RpcNamespace::Admin) => map_admin_requests(req, context).await,
        Ok(RpcNamespace::Debug) => map_debug_requests(req, context).await,
        Ok(RpcNamespace::Web3) => map_web3_requests(req, context),
        Ok(RpcNamespace::Net) => map_net_requests(req, context).await,
        Ok(RpcNamespace::Mempool) => map_mempool_requests(req, context).await,
        Ok(RpcNamespace::Engine) => Err(RpcErr::Internal(
            "Engine namespace not allowed in map_http_requests".to_owned(),
        )),
        Err(rpc_err) => Err(rpc_err),
    }
}

/// Handle requests from consensus client
pub async fn map_authrpc_requests(
    req: &RpcRequest,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    match req.namespace() {
        Ok(RpcNamespace::Engine) => map_engine_requests(req, context).await,
        Ok(RpcNamespace::Eth) => map_eth_requests(req, context).await,
        _ => Err(RpcErr::MethodNotFound(req.method.clone())),
    }
}

pub async fn map_eth_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "eth_chainId" => ChainId::call(req, context).await,
        "eth_syncing" => Syncing::call(req, context).await,
        "eth_getBlockByNumber" => GetBlockByNumberRequest::call(req, context).await,
        "eth_getBlockByHash" => GetBlockByHashRequest::call(req, context).await,
        "eth_getBalance" => GetBalanceRequest::call(req, context).await,
        "eth_getCode" => GetCodeRequest::call(req, context).await,
        "eth_getStorageAt" => GetStorageAtRequest::call(req, context).await,
        "eth_getBlockTransactionCountByNumber" => {
            GetBlockTransactionCountRequest::call(req, context).await
        }
        "eth_getBlockTransactionCountByHash" => {
            GetBlockTransactionCountRequest::call(req, context).await
        }
        "eth_getTransactionByBlockNumberAndIndex" => {
            GetTransactionByBlockNumberAndIndexRequest::call(req, context).await
        }
        "eth_getTransactionByBlockHashAndIndex" => {
            GetTransactionByBlockHashAndIndexRequest::call(req, context).await
        }
        "eth_getBlockReceipts" => GetBlockReceiptsRequest::call(req, context).await,
        "eth_getTransactionByHash" => GetTransactionByHashRequest::call(req, context).await,
        "eth_getTransactionReceipt" => GetTransactionReceiptRequest::call(req, context).await,
        "eth_createAccessList" => CreateAccessListRequest::call(req, context).await,
        "eth_blockNumber" => BlockNumberRequest::call(req, context).await,
        "eth_call" => CallRequest::call(req, context).await,
        "eth_blobBaseFee" => GetBlobBaseFee::call(req, context).await,
        "eth_getTransactionCount" => GetTransactionCountRequest::call(req, context).await,
        "eth_feeHistory" => FeeHistoryRequest::call(req, context).await,
        "eth_estimateGas" => EstimateGasRequest::call(req, context).await,
        "eth_getLogs" => LogsFilter::call(req, context).await,
        "eth_newFilter" => {
            NewFilterRequest::stateful_call(req, context.storage, context.active_filters).await
        }
        "eth_uninstallFilter" => {
            DeleteFilterRequest::stateful_call(req, context.storage, context.active_filters)
        }
        "eth_getFilterChanges" => {
            FilterChangesRequest::stateful_call(req, context.storage, context.active_filters).await
        }
        "eth_sendRawTransaction" => SendRawTransactionRequest::call(req, context).await,
        "eth_getProof" => GetProofRequest::call(req, context).await,
        "eth_gasPrice" => GasPrice::call(req, context).await,
        "eth_maxPriorityFeePerGas" => {
            eth::max_priority_fee::MaxPriorityFee::call(req, context).await
        }
        "eth_config" => Config::call(req, context).await,
        unknown_eth_method => Err(RpcErr::MethodNotFound(unknown_eth_method.to_owned())),
    }
}

pub async fn map_debug_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "debug_getRawHeader" => GetRawHeaderRequest::call(req, context).await,
        "debug_getRawBlock" => GetRawBlockRequest::call(req, context).await,
        "debug_getRawTransaction" => GetRawTransaction::call(req, context).await,
        "debug_getRawReceipts" => GetRawReceipts::call(req, context).await,
        "debug_executionWitness" => ExecutionWitnessRequest::call(req, context).await,
        "debug_traceTransaction" => TraceTransactionRequest::call(req, context).await,
        "debug_traceBlockByNumber" => TraceBlockByNumberRequest::call(req, context).await,
        unknown_debug_method => Err(RpcErr::MethodNotFound(unknown_debug_method.to_owned())),
    }
}

pub async fn map_engine_requests(
    req: &RpcRequest,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "engine_exchangeCapabilities" => ExchangeCapabilitiesRequest::call(req, context).await,
        "engine_forkchoiceUpdatedV1" => ForkChoiceUpdatedV1::call(req, context).await,
        "engine_forkchoiceUpdatedV2" => ForkChoiceUpdatedV2::call(req, context).await,
        "engine_forkchoiceUpdatedV3" => ForkChoiceUpdatedV3::call(req, context).await,
        "engine_newPayloadV4" => NewPayloadV4Request::call(req, context).await,
        "engine_newPayloadV3" => NewPayloadV3Request::call(req, context).await,
        "engine_newPayloadV2" => NewPayloadV2Request::call(req, context).await,
        "engine_newPayloadV1" => NewPayloadV1Request::call(req, context).await,
        "engine_exchangeTransitionConfigurationV1" => {
            ExchangeTransitionConfigV1Req::call(req, context).await
        }
        "engine_getPayloadV4" => GetPayloadV4Request::call(req, context).await,
        "engine_getPayloadV3" => GetPayloadV3Request::call(req, context).await,
        "engine_getPayloadV2" => GetPayloadV2Request::call(req, context).await,
        "engine_getPayloadV1" => GetPayloadV1Request::call(req, context).await,
        "engine_getPayloadBodiesByHashV1" => {
            GetPayloadBodiesByHashV1Request::call(req, context).await
        }
        "engine_getPayloadBodiesByRangeV1" => {
            GetPayloadBodiesByRangeV1Request::call(req, context).await
        }
        "engine_getBlobsV1" => BlobsV1Request::call(req, context).await,
        unknown_engine_method => Err(RpcErr::MethodNotFound(unknown_engine_method.to_owned())),
    }
}

pub async fn map_admin_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "admin_nodeInfo" => admin::node_info(context.storage, &context.node_data),
        "admin_peers" => admin::peers(&context).await,
        "admin_setLogLevel" => admin::set_log_level(req, &context.log_filter_handler).await,
        unknown_admin_method => Err(RpcErr::MethodNotFound(unknown_admin_method.to_owned())),
    }
}

pub fn map_web3_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "web3_clientVersion" => Ok(Value::String(context.node_data.client_version)),
        unknown_web3_method => Err(RpcErr::MethodNotFound(unknown_web3_method.to_owned())),
    }
}

pub async fn map_net_requests(req: &RpcRequest, contex: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "net_version" => net::version(req, contex),
        "net_peerCount" => net::peer_count(req, contex).await,
        unknown_net_method => Err(RpcErr::MethodNotFound(unknown_net_method.to_owned())),
    }
}

pub async fn map_mempool_requests(
    req: &RpcRequest,
    contex: RpcApiContext,
) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        // TODO: The endpoint name matches geth's endpoint for compatibility, consider changing it in the future
        "txpool_content" => mempool::content(contex).await,
        "txpool_status" => mempool::status(contex).await,
        unknown_mempool_method => Err(RpcErr::MethodNotFound(unknown_mempool_method.to_owned())),
    }
}

pub fn rpc_response<E>(id: RpcRequestId, res: Result<Value, E>) -> Result<Value, RpcErr>
where
    E: Into<RpcErrorMetadata>,
{
    Ok(match res {
        Ok(result) => serde_json::to_value(RpcSuccessResponse {
            id,
            jsonrpc: "2.0".to_string(),
            result,
        }),
        Err(error) => serde_json::to_value(RpcErrorResponse {
            id,
            jsonrpc: "2.0".to_string(),
            error: error.into(),
        }),
    }?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::default_context_with_storage;
    use ethrex_common::{
        H160,
        types::{ChainConfig, Genesis},
    };
    use ethrex_storage::{EngineType, Store};
    use sha3::{Digest, Keccak256};
    use std::io::BufReader;
    use std::str::FromStr;
    use std::{fs::File, path::Path};

    // Maps string rpc response to RpcSuccessResponse as serde Value
    // This is used to avoid failures due to field order and allow easier string comparisons for responses
    fn to_rpc_response_success_value(str: &str) -> serde_json::Value {
        serde_json::to_value(serde_json::from_str::<RpcSuccessResponse>(str).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn admin_nodeinfo_request() {
        let body = r#"{"jsonrpc":"2.0", "method":"admin_nodeInfo", "params":[], "id":1}"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        storage
            .set_chain_config(&example_chain_config())
            .await
            .unwrap();
        let context = default_context_with_storage(storage).await;
        let local_p2p_node = context.node_data.local_p2p_node.clone();

        let enr_url = context.node_data.local_node_record.enr_url().unwrap();
        let result = map_http_requests(&request, context).await;
        let rpc_response = rpc_response(request.id, result).unwrap();
        let blob_schedule = serde_json::json!({
            "cancun": { "target": 3, "max": 6, "baseFeeUpdateFraction": 3338477 },
            "prague": { "target": 6, "max": 9, "baseFeeUpdateFraction": 5007716 },
            "osaka": { "target": 6, "max": 9, "baseFeeUpdateFraction": 5007716 },
            "bpo1": { "target": 10, "max": 15, "baseFeeUpdateFraction": 8346193 },
            "bpo2": { "target": 14, "max": 21, "baseFeeUpdateFraction": 11684671 },
        });
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "enode": "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@127.0.0.1:30303",
                "enr": enr_url,
                "id": hex::encode(Keccak256::digest(local_p2p_node.public_key)),
                "ip": "127.0.0.1",
                "name": "ethrex/test",
                "ports": {
                    "discovery": 30303,
                    "listener": 30303
                },
                "protocols": {
                    "eth": {
                        "chainId": 3151908,
                        "homesteadBlock": 0,
                        "daoForkBlock": null,
                        "daoForkSupport": false,
                        "eip150Block": 0,
                        "eip155Block": 0,
                        "eip158Block": 0,
                        "byzantiumBlock": 0,
                        "constantinopleBlock": 0,
                        "petersburgBlock": 0,
                        "istanbulBlock": 0,
                        "muirGlacierBlock": null,
                        "berlinBlock": 0,
                        "londonBlock": 0,
                        "arrowGlacierBlock": null,
                        "grayGlacierBlock": null,
                        "mergeNetsplitBlock": 0,
                        "shanghaiTime": 0,
                        "cancunTime": 0,
                        "pragueTime": 1718232101,
                        "verkleTime": null,
                        "osakaTime": null,
                        "bpo1Time": null,
                        "bpo2Time": null,
                        "bpo3Time": null,
                        "bpo4Time": null,
                        "bpo5Time": null,
                        "terminalTotalDifficulty": 0,
                        "terminalTotalDifficultyPassed": true,
                        "blobSchedule": blob_schedule,
                        "depositContractAddress": H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa").unwrap(),
                        "enableVerkleAtGenesis": false,
                    }
                },
            }
        });
        let expected_response = to_rpc_response_success_value(&json.to_string());
        assert_eq!(rpc_response.to_string(), expected_response.to_string())
    }

    // Reads genesis file taken from https://github.com/ethereum/execution-apis/blob/main/tests/genesis.json
    fn read_execution_api_genesis_file() -> Genesis {
        let file = File::open("../../../fixtures/genesis/execution-api.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file")
    }

    #[tokio::test]
    async fn create_access_list_simple_transfer() {
        // Create Request
        // Request taken from https://github.com/ethereum/execution-apis/blob/main/tests/eth_createAccessList/create-al-value-transfer.io
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"eth_createAccessList","params":[{"from":"0x0c2c51a0990aee1d73c1228de158688341557508","nonce":"0x0","to":"0x0100000000000000000000000000000000000000","value":"0xa"},"0x00"]}"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();
        // Setup initial storage
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let genesis = read_execution_api_genesis_file();
        storage
            .add_initial_state(genesis)
            .await
            .expect("Failed to add genesis block to DB");
        // Process request
        let context = default_context_with_storage(storage).await;
        let result = map_http_requests(&request, context).await;
        let response = rpc_response(request.id, result).unwrap();
        let expected_response = to_rpc_response_success_value(
            r#"{"jsonrpc":"2.0","id":1,"result":{"accessList":[],"gasUsed":"0x5208"}}"#,
        );
        assert_eq!(response.to_string(), expected_response.to_string());
    }

    fn example_chain_config() -> ChainConfig {
        ChainConfig {
            chain_id: 3151908_u64,
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            merge_netsplit_block: Some(0),
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(1718232101),
            terminal_total_difficulty: Some(0),
            terminal_total_difficulty_passed: true,
            deposit_contract_address: H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa")
                .unwrap(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn net_version_test() {
        let body = r#"{"jsonrpc":"2.0","method":"net_version","params":[],"id":67}"#;
        let request: RpcRequest = serde_json::from_str(body).expect("serde serialization failed");
        // Setup initial storage
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        storage
            .set_chain_config(&example_chain_config())
            .await
            .unwrap();
        let chain_id = storage
            .get_chain_config()
            .expect("failed to get chain_id")
            .chain_id
            .to_string();
        let context = default_context_with_storage(storage).await;
        // Process request
        let result = map_http_requests(&request, context).await;
        let response = rpc_response(request.id, result).unwrap();
        let expected_response_string =
            format!(r#"{{"id":67,"jsonrpc": "2.0","result": "{chain_id}"}}"#);
        let expected_response = to_rpc_response_success_value(&expected_response_string);
        assert_eq!(response.to_string(), expected_response.to_string());
    }

    #[tokio::test]
    async fn eth_config_request_cancun_with_prague_scheduled() {
        let body = r#"{"jsonrpc":"2.0", "method":"eth_config", "params":[], "id":1}"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();
        let storage = Store::new_from_genesis(
            Path::new("temp.db"),
            EngineType::InMemory,
            "../../../cmd/ethrex/networks/hoodi/genesis.json",
        )
        .await
        .expect("Failed to create test DB");
        let context = default_context_with_storage(storage).await;
        let result = map_http_requests(&request, context).await;
        let rpc_response = rpc_response(request.id, result).unwrap();
        let json = serde_json::json!({
            "id": 1,
            "jsonrpc": "2.0",
            "result": {
                "current": {
                    "activationTime": 0,
                    "blobSchedule": {
                        "baseFeeUpdateFraction": 3338477,
                        "max": 6,
                        "target": 3
                    },
                    "chainId": "0x88bb0",
                    "forkId": "0xbef71d30",
                    "precompiles": {
                        "BLAKE2F": "0x0000000000000000000000000000000000000009",
                        "BN254_ADD": "0x0000000000000000000000000000000000000006",
                        "BN254_MUL": "0x0000000000000000000000000000000000000007",
                        "BN254_PAIRING": "0x0000000000000000000000000000000000000008",
                        "ECREC": "0x0000000000000000000000000000000000000001",
                        "ID": "0x0000000000000000000000000000000000000004",
                        "KZG_POINT_EVALUATION": "0x000000000000000000000000000000000000000a",
                        "MODEXP": "0x0000000000000000000000000000000000000005",
                        "RIPEMD160": "0x0000000000000000000000000000000000000003",
                        "SHA256": "0x0000000000000000000000000000000000000002"
                    },
                    "systemContracts": {
                        "BEACON_ROOTS_ADDRESS": "0x000f3df6d732807ef1319fb7b8bb8522d0beac02"
                    }
                },
                "next": {
                    "activationTime": 1742999832,
                    "blobSchedule": {
                        "baseFeeUpdateFraction": 5007716,
                        "max": 9,
                        "target": 6
                    },
                    "chainId": "0x88bb0",
                    "forkId": "0x0929e24e",
                    "precompiles": {
                        "BLAKE2F": "0x0000000000000000000000000000000000000009",
                        "BLS12_G1ADD": "0x000000000000000000000000000000000000000b",
                        "BLS12_G1MSM": "0x000000000000000000000000000000000000000c",
                        "BLS12_G2ADD": "0x000000000000000000000000000000000000000d",
                        "BLS12_G2MSM": "0x000000000000000000000000000000000000000e",
                        "BLS12_MAP_FP2_TO_G2": "0x0000000000000000000000000000000000000011",
                        "BLS12_MAP_FP_TO_G1": "0x0000000000000000000000000000000000000010",
                        "BLS12_PAIRING_CHECK": "0x000000000000000000000000000000000000000f",
                        "BN254_ADD": "0x0000000000000000000000000000000000000006",
                        "BN254_MUL": "0x0000000000000000000000000000000000000007",
                        "BN254_PAIRING": "0x0000000000000000000000000000000000000008",
                        "ECREC": "0x0000000000000000000000000000000000000001",
                        "ID": "0x0000000000000000000000000000000000000004",
                        "KZG_POINT_EVALUATION": "0x000000000000000000000000000000000000000a",
                        "MODEXP": "0x0000000000000000000000000000000000000005",
                        "RIPEMD160": "0x0000000000000000000000000000000000000003",
                        "SHA256": "0x0000000000000000000000000000000000000002"
                    },
                    "systemContracts": {
                        "BEACON_ROOTS_ADDRESS": "0x000f3df6d732807ef1319fb7b8bb8522d0beac02",
                        "CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS": "0x0000bbddc7ce488642fb579f8b00f3a590007251",
                        "DEPOSIT_CONTRACT_ADDRESS": "0x00000000219ab540356cbb839cbe05303d7705fa",
                        "HISTORY_STORAGE_ADDRESS": "0x0000f90827f1c53a10cb7a02335b175320002935",
                        "WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS": "0x00000961ef480eb55e80d19ad83579a64c007002"
                    }
                },
                "last": {
                    "activationTime": 1762955544,
                    "blobSchedule": {
                        "baseFeeUpdateFraction": 11684671,
                        "max": 21,
                        "target": 14,
                    },
                    "chainId": "0x88bb0",
                    "forkId": "0x23aa1351",
                    "precompiles": {
                        "BLAKE2F": "0x0000000000000000000000000000000000000009",
                        "BLS12_G1ADD": "0x000000000000000000000000000000000000000b",
                        "BLS12_G1MSM": "0x000000000000000000000000000000000000000c",
                        "BLS12_G2ADD": "0x000000000000000000000000000000000000000d",
                        "BLS12_G2MSM": "0x000000000000000000000000000000000000000e",
                        "BLS12_MAP_FP2_TO_G2": "0x0000000000000000000000000000000000000011",
                        "BLS12_MAP_FP_TO_G1": "0x0000000000000000000000000000000000000010",
                        "BLS12_PAIRING_CHECK": "0x000000000000000000000000000000000000000f",
                        "BN254_ADD": "0x0000000000000000000000000000000000000006",
                        "BN254_MUL": "0x0000000000000000000000000000000000000007",
                        "BN254_PAIRING": "0x0000000000000000000000000000000000000008",
                        "ECREC": "0x0000000000000000000000000000000000000001",
                        "ID": "0x0000000000000000000000000000000000000004",
                        "KZG_POINT_EVALUATION": "0x000000000000000000000000000000000000000a",
                        "MODEXP": "0x0000000000000000000000000000000000000005",
                        "P256_VERIFICATION":"0x0000000000000000000000000000000000000100",
                        "RIPEMD160": "0x0000000000000000000000000000000000000003",
                        "SHA256": "0x0000000000000000000000000000000000000002"
                    },
                    "systemContracts": {
                        "BEACON_ROOTS_ADDRESS": "0x000f3df6d732807ef1319fb7b8bb8522d0beac02",
                        "CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS": "0x0000bbddc7ce488642fb579f8b00f3a590007251",
                        "DEPOSIT_CONTRACT_ADDRESS": "0x00000000219ab540356cbb839cbe05303d7705fa",
                        "HISTORY_STORAGE_ADDRESS": "0x0000f90827f1c53a10cb7a02335b175320002935",
                        "WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS": "0x00000961ef480eb55e80d19ad83579a64c007002"
                    }
                },
            }
        });
        let expected_response = to_rpc_response_success_value(&json.to_string());
        assert_eq!(rpc_response.to_string(), expected_response.to_string())
    }
}
