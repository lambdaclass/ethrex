use crate::authentication::authenticate;
use crate::debug::execution_witness::ExecutionWitnessRequest;
use crate::engine::blobs::{BlobsV2Request, BlobsV3Request};
use crate::engine::client_version::GetClientVersionV1Request;
use crate::engine::payload::{GetPayloadV5Request, NewPayloadV5Request};
use crate::engine::{
    ExchangeCapabilitiesRequest,
    blobs::BlobsV1Request,
    exchange_transition_config::ExchangeTransitionConfigV1Req,
    fork_choice::{
        ForkChoiceUpdatedV1, ForkChoiceUpdatedV2, ForkChoiceUpdatedV3, ForkChoiceUpdatedV4,
    },
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
use axum::extract::ws::WebSocket;
use axum::extract::{DefaultBodyLimit, State, WebSocketUpgrade};
use axum::{Json, Router, http::StatusCode, routing::post};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_blockchain::error::ChainError;
use ethrex_common::types::Block;
use ethrex_metrics::rpc::{RpcOutcome, record_async_duration, record_rpc_outcome};
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
use tokio::net::TcpListener;
use tokio::sync::{
    Mutex as TokioMutex,
    mpsc::{UnboundedSender, unbounded_channel},
    oneshot,
};
use tokio::time::timeout;
use tower_http::cors::CorsLayer;
use tracing::{error, info, warn};
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

/// Wrapper for JSON-RPC requests that can be either single or batched.
///
/// According to the JSON-RPC 2.0 specification, clients may send either a single
/// request object or an array of request objects (batch request).
#[derive(Deserialize)]
#[serde(untagged)]
pub enum RpcRequestWrapper {
    /// A single JSON-RPC request.
    Single(RpcRequest),
    /// A batch of JSON-RPC requests to be processed together.
    Multiple(Vec<RpcRequest>),
}

/// Shared context passed to all RPC request handlers.
///
/// This struct contains all the dependencies that RPC handlers need to process requests,
/// including storage access, blockchain state, P2P networking, and configuration.
///
/// The context is cloned for each request, with most fields being cheap `Arc` references.
#[derive(Debug, Clone)]
pub struct RpcApiContext {
    /// Database storage for blocks, transactions, and state.
    pub storage: Store,
    /// Blockchain instance for block validation and execution.
    pub blockchain: Arc<Blockchain>,
    /// Active log filters for `eth_newFilter` / `eth_getFilterChanges` endpoints.
    pub active_filters: ActiveFilters,
    /// Sync manager for coordinating block synchronization (None for L2 nodes).
    pub syncer: Option<Arc<SyncManager>>,
    /// Peer handler for P2P network operations (None for L2 nodes).
    pub peer_handler: Option<PeerHandler>,
    /// Node identity and configuration data.
    pub node_data: NodeData,
    /// Gas tip estimator for `eth_gasPrice` and `eth_maxPriorityFeePerGas`.
    pub gas_tip_estimator: Arc<TokioMutex<GasTipEstimator>>,
    /// Handler for dynamically changing log filter levels via `admin_setLogLevel`.
    pub log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    /// Maximum gas limit for blocks (used in payload building).
    pub gas_ceil: u64,
    /// Channel for sending blocks to the block executor worker thread.
    pub block_worker_channel: UnboundedSender<(oneshot::Sender<Result<(), ChainError>>, Block)>,
}

/// Client version information used for identification in the Engine API and P2P.
///
/// This struct contains the individual components of the client version, which are
/// used by `engine_getClientVersionV1` and other identification endpoints.
///
/// Implements `Display` to return the pre-formatted version string.
#[derive(Debug, Clone)]
pub struct ClientVersion {
    /// Client name (e.g., "ethrex").
    pub name: String,
    /// Semantic version (e.g., "0.1.0").
    pub version: String,
    /// Git branch name (e.g., "main").
    pub branch: String,
    /// Git commit hash (full SHA).
    pub commit: String,
    /// OS and architecture (e.g., "x86_64-apple-darwin").
    pub os_arch: String,
    /// Rust compiler version (e.g., "1.70.0").
    pub rustc_version: String,
    /// Pre-formatted version string for efficient Display.
    formatted: String,
}

impl ClientVersion {
    /// Creates a new ClientVersion with all fields and a pre-formatted string.
    pub fn new(
        name: String,
        version: String,
        branch: String,
        commit: String,
        os_arch: String,
        rustc_version: String,
    ) -> Self {
        let formatted = format!(
            "{}/v{}-{}-{}/{}/rustc-v{}",
            name, version, branch, commit, os_arch, rustc_version
        );
        Self {
            name,
            version,
            branch,
            commit,
            os_arch,
            rustc_version,
            formatted,
        }
    }
}

impl std::fmt::Display for ClientVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.formatted)
    }
}

/// Node identity and configuration information.
///
/// Contains the node's cryptographic identity, network endpoints, and metadata
/// used for P2P discovery and RPC responses.
#[derive(Debug, Clone)]
pub struct NodeData {
    /// JWT secret for authenticating Engine API requests from consensus clients.
    pub jwt_secret: Bytes,
    /// Local P2P node identity (public key and address).
    pub local_p2p_node: Node,
    /// ENR (Ethereum Node Record) for node discovery.
    pub local_node_record: NodeRecord,
    /// Client version information.
    pub client_version: ClientVersion,
    /// Extra data included in mined blocks.
    pub extra_data: Bytes,
}

/// Trait for implementing JSON-RPC method handlers.
///
/// Each RPC method (e.g., `eth_getBalance`, `engine_newPayloadV3`) is implemented
/// as a struct that implements this trait. The trait provides a standard pattern
/// for parsing parameters and handling requests.
///
/// # Example
///
/// ```ignore
/// struct GetBalanceRequest {
///     address: Address,
///     block: BlockId,
/// }
///
/// impl RpcHandler for GetBalanceRequest {
///     fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
///         let params = params.as_ref().ok_or(RpcErr::MissingParam("params"))?;
///         Ok(Self {
///             address: serde_json::from_value(params[0].clone())?,
///             block: serde_json::from_value(params[1].clone())?,
///         })
///     }
///
///     async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
///         let balance = context.storage.get_balance(self.address, self.block)?;
///         Ok(serde_json::to_value(balance)?)
///     }
/// }
/// ```
#[allow(async_fn_in_trait)]
pub trait RpcHandler: Sized {
    /// Parse JSON-RPC parameters into the handler struct.
    ///
    /// Returns an error if required parameters are missing or have invalid types.
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr>;

    /// Entry point for handling an RPC request.
    ///
    /// This method parses the request, records metrics, and delegates to `handle()`.
    /// Most implementations should not override this method.
    async fn call(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        let namespace = match req.namespace() {
            Ok(RpcNamespace::Engine) => "engine",
            _ => "rpc",
        };
        let method = req.method.as_str();

        let result =
            record_async_duration(
                namespace,
                method,
                async move { request.handle(context).await },
            )
            .await;

        let outcome = match &result {
            Ok(_) => RpcOutcome::Success,
            Err(err) => RpcOutcome::Error(get_error_kind(err)),
        };
        record_rpc_outcome(namespace, method, outcome);

        result
    }

    /// Handle the RPC request and return a JSON response.
    ///
    /// This is where the actual business logic for the RPC method lives.
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr>;
}

fn get_error_kind(err: &RpcErr) -> &'static str {
    match err {
        RpcErr::MethodNotFound(_) => "MethodNotFound",
        RpcErr::WrongParam(_) => "WrongParam",
        RpcErr::BadParams(_) => "BadParams",
        RpcErr::MissingParam(_) => "MissingParam",
        RpcErr::TooLargeRequest => "TooLargeRequest",
        RpcErr::BadHexFormat(_) => "BadHexFormat",
        RpcErr::UnsuportedFork(_) => "UnsuportedFork",
        RpcErr::Internal(_) => "Internal",
        RpcErr::Vm(_) => "Vm",
        RpcErr::Revert { .. } => "Revert",
        RpcErr::Halt { .. } => "Halt",
        RpcErr::AuthenticationError(_) => "AuthenticationError",
        RpcErr::InvalidForkChoiceState(_) => "InvalidForkChoiceState",
        RpcErr::InvalidPayloadAttributes(_) => "InvalidPayloadAttributes",
        RpcErr::UnknownPayload(_) => "UnknownPayload",
    }
}

/// Duration after which inactive filters are cleaned up.
///
/// Filters created via `eth_newFilter` are automatically removed if not
/// accessed within this duration. In tests, this is set to 1 second for
/// faster test execution.
pub const FILTER_DURATION: Duration = {
    if cfg!(test) {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5 * 60)
    }
};

/// Spawns a dedicated thread for sequential block execution.
///
/// Blocks received from the consensus client via `engine_newPayload` are sent
/// to this worker thread for execution. This ensures blocks are processed
/// sequentially and prevents the async runtime from being blocked by CPU-intensive
/// block execution.
///
/// # Returns
///
/// An unbounded channel sender for submitting blocks. Each submission includes
/// a oneshot channel for receiving the execution result.
///
/// # Panics
///
/// Panics if the worker thread cannot be spawned.
pub fn start_block_executor(
    blockchain: Arc<Blockchain>,
) -> UnboundedSender<(oneshot::Sender<Result<(), ChainError>>, Block)> {
    let (block_worker_channel, mut block_receiver) =
        unbounded_channel::<(oneshot::Sender<Result<(), ChainError>>, Block)>();
    std::thread::Builder::new()
        .name("block_executor".to_string())
        .spawn(move || {
            while let Some((notify, block)) = block_receiver.blocking_recv() {
                let _ = notify
                    .send(blockchain.add_block_pipeline(block))
                    .inspect_err(|_| tracing::error!("failed to notify caller"));
            }
        })
        .expect("Falied to spawn block_executor thread");
    block_worker_channel
}

/// Starts the JSON-RPC API servers.
///
/// This function initializes and runs three server endpoints:
///
/// 1. **HTTP Server** (`http_addr`): Public JSON-RPC endpoint for standard Ethereum
///    methods (`eth_*`, `debug_*`, `net_*`, `admin_*`, `web3_*`, `txpool_*`).
///
/// 2. **WebSocket Server** (`ws_addr`): Optional WebSocket endpoint for the same
///    methods as HTTP, enabling persistent connections.
///
/// 3. **Auth RPC Server** (`authrpc_addr`): JWT-authenticated endpoint for Engine API
///    methods (`engine_*`) used by consensus clients.
///
/// # Arguments
///
/// * `http_addr` - Socket address for the HTTP server (e.g., `127.0.0.1:8545`)
/// * `ws_addr` - Optional socket address for WebSocket server
/// * `authrpc_addr` - Socket address for authenticated Engine API (e.g., `127.0.0.1:8551`)
/// * `storage` - Database storage instance
/// * `blockchain` - Blockchain instance for block operations
/// * `jwt_secret` - JWT secret for Engine API authentication
/// * `local_p2p_node` - Local node identity for P2P networking
/// * `local_node_record` - ENR for node discovery
/// * `syncer` - Sync manager for block synchronization
/// * `peer_handler` - Handler for P2P peer operations
/// * `client_version` - Client version information for `web3_clientVersion` and `engine_getClientVersionV1`
/// * `log_filter_handler` - Optional handler for dynamic log level changes
/// * `gas_ceil` - Maximum gas limit for payload building
/// * `extra_data` - Extra data to include in mined blocks
///
/// # Errors
///
/// Returns an error if any server fails to bind to its address.
///
/// # Shutdown
///
/// All servers shut down gracefully on SIGINT (Ctrl+C).
#[allow(clippy::too_many_arguments)]
pub async fn start_api(
    http_addr: SocketAddr,
    ws_addr: Option<SocketAddr>,
    authrpc_addr: SocketAddr,
    storage: Store,
    blockchain: Arc<Blockchain>,
    jwt_secret: Bytes,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    syncer: SyncManager,
    peer_handler: PeerHandler,
    client_version: ClientVersion,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    gas_ceil: u64,
    extra_data: String,
) -> Result<(), RpcErr> {
    // TODO: Refactor how filters are handled,
    // filters are used by the filters endpoints (eth_newFilter, eth_getFilterChanges, ...etc)
    let active_filters = Arc::new(Mutex::new(HashMap::new()));
    let block_worker_channel = start_block_executor(blockchain.clone());
    let service_context = RpcApiContext {
        storage,
        blockchain,
        active_filters: active_filters.clone(),
        syncer: Some(Arc::new(syncer)),
        peer_handler: Some(peer_handler),
        node_data: NodeData {
            jwt_secret,
            local_p2p_node,
            local_node_record,
            client_version,
            extra_data: extra_data.into(),
        },
        gas_tip_estimator: Arc::new(TokioMutex::new(GasTipEstimator::new())),
        log_filter_handler,
        gas_ceil,
        block_worker_channel,
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
        .layer(cors.clone())
        .with_state(service_context.clone());
    let http_listener = TcpListener::bind(http_addr)
        .await
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    let http_server = axum::serve(http_listener, http_router)
        .with_graceful_shutdown(shutdown_signal())
        .into_future();
    info!("Starting HTTP server at {http_addr}");

    let (timer_sender, mut timer_receiver) = tokio::sync::watch::channel(());

    tokio::spawn(async move {
        loop {
            let result = timeout(Duration::from_secs(30), timer_receiver.changed()).await;
            if result.is_err() {
                warn!("No messages from the consensus layer. Is the consensus client running?");
            }
        }
    });

    let authrpc_handler = move |ctx, auth, body| async move {
        let _ = timer_sender.send(());
        handle_authrpc_request(ctx, auth, body).await
    };

    let authrpc_router = Router::new()
        .route("/", post(authrpc_handler))
        .with_state(service_context.clone())
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

    if let Some(address) = ws_addr {
        let ws_handler = |ws: WebSocketUpgrade, ctx| async {
            ws.on_upgrade(|socket| handle_websocket(socket, ctx))
        };
        let ws_router = Router::new()
            .route("/", axum::routing::any(ws_handler))
            .layer(cors)
            .with_state(service_context);
        let ws_listener = TcpListener::bind(address)
            .await
            .map_err(|error| RpcErr::Internal(error.to_string()))?;
        let ws_server = axum::serve(ws_listener, ws_router)
            .with_graceful_shutdown(shutdown_signal())
            .into_future();
        info!("Starting WS server at {address}");

        let _ = tokio::try_join!(authrpc_server, http_server, ws_server)
            .inspect_err(|e| error!("Error shutting down servers: {e:?}"));
    } else {
        let _ = tokio::try_join!(authrpc_server, http_server)
            .inspect_err(|e| error!("Error shutting down servers: {e:?}"));
    }

    Ok(())
}

/// Returns a future that completes when SIGINT (Ctrl+C) is received.
///
/// Used to implement graceful shutdown for all RPC servers.
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

async fn handle_websocket(mut socket: WebSocket, state: State<RpcApiContext>) {
    while let Some(message) = socket.recv().await {
        let Ok(body) = message
            .and_then(|msg| msg.into_text())
            .map(|msg| msg.to_string())
        else {
            return;
        };

        // ok-clone: increase arc reference count
        let Ok(response) = handle_http_request(state.clone(), body)
            .await
            .map(|res| res.to_string())
        else {
            return;
        };

        if socket.send(response.into()).await.is_err() {
            return;
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
        Ok(RpcNamespace::Mempool) => map_mempool_requests(req, context),
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

/// Routes `eth_*` namespace requests to their handlers.
///
/// Handles all standard Ethereum JSON-RPC methods including:
/// - Account queries: `eth_getBalance`, `eth_getCode`, `eth_getStorageAt`, `eth_getTransactionCount`
/// - Block queries: `eth_getBlockByNumber`, `eth_getBlockByHash`, `eth_blockNumber`
/// - Transaction operations: `eth_sendRawTransaction`, `eth_getTransactionByHash`, `eth_getTransactionReceipt`
/// - Gas estimation: `eth_estimateGas`, `eth_gasPrice`, `eth_maxPriorityFeePerGas`, `eth_feeHistory`
/// - Filters: `eth_newFilter`, `eth_getFilterChanges`, `eth_uninstallFilter`, `eth_getLogs`
/// - Misc: `eth_chainId`, `eth_syncing`, `eth_createAccessList`, `eth_getProof`
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

/// Routes `debug_*` namespace requests to their handlers.
///
/// Handles debugging and introspection methods:
/// - Raw data: `debug_getRawHeader`, `debug_getRawBlock`, `debug_getRawTransaction`, `debug_getRawReceipts`
/// - Execution witness: `debug_executionWitness` (for stateless validation)
/// - Tracing: `debug_traceTransaction`, `debug_traceBlockByNumber`
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

/// Routes `engine_*` namespace requests to their handlers.
///
/// These are Engine API methods used by consensus clients (e.g., Lighthouse, Prysm)
/// to communicate with the execution layer. All methods require JWT authentication.
///
/// Handles:
/// - Fork choice: `engine_forkchoiceUpdatedV1/V2/V3`
/// - Payload submission: `engine_newPayloadV1/V2/V3/V4`
/// - Payload retrieval: `engine_getPayloadV1/V2/V3/V4/V5`
/// - Payload bodies: `engine_getPayloadBodiesByHashV1`, `engine_getPayloadBodiesByRangeV1`
/// - Blob retrieval: `engine_getBlobsV1/V2/V3`
/// - Capabilities: `engine_exchangeCapabilities`, `engine_exchangeTransitionConfigurationV1`
pub async fn map_engine_requests(
    req: &RpcRequest,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "engine_exchangeCapabilities" => ExchangeCapabilitiesRequest::call(req, context).await,
        "engine_forkchoiceUpdatedV1" => ForkChoiceUpdatedV1::call(req, context).await,
        "engine_forkchoiceUpdatedV2" => ForkChoiceUpdatedV2::call(req, context).await,
        "engine_forkchoiceUpdatedV3" => ForkChoiceUpdatedV3::call(req, context).await,
        "engine_forkchoiceUpdatedV4" => ForkChoiceUpdatedV4::call(req, context).await,
        "engine_newPayloadV5" => NewPayloadV5Request::call(req, context).await,
        "engine_newPayloadV4" => NewPayloadV4Request::call(req, context).await,
        "engine_newPayloadV3" => NewPayloadV3Request::call(req, context).await,
        "engine_newPayloadV2" => NewPayloadV2Request::call(req, context).await,
        "engine_newPayloadV1" => NewPayloadV1Request::call(req, context).await,
        "engine_exchangeTransitionConfigurationV1" => {
            ExchangeTransitionConfigV1Req::call(req, context).await
        }
        "engine_getPayloadV5" => GetPayloadV5Request::call(req, context).await,
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
        "engine_getBlobsV2" => BlobsV2Request::call(req, context).await,
        "engine_getBlobsV3" => BlobsV3Request::call(req, context).await,
        "engine_getClientVersionV1" => GetClientVersionV1Request::call(req, context).await,
        unknown_engine_method => Err(RpcErr::MethodNotFound(unknown_engine_method.to_owned())),
    }
}

pub async fn map_admin_requests(
    req: &RpcRequest,
    mut context: RpcApiContext,
) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "admin_nodeInfo" => admin::node_info(context.storage, &context.node_data),
        "admin_peers" => admin::peers(&mut context).await,
        "admin_setLogLevel" => admin::set_log_level(req, &context.log_filter_handler),
        "admin_addPeer" => admin::add_peer(&mut context, req).await,
        unknown_admin_method => Err(RpcErr::MethodNotFound(unknown_admin_method.to_owned())),
    }
}

pub fn map_web3_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "web3_clientVersion" => Ok(Value::String(context.node_data.client_version.to_string())),
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

pub fn map_mempool_requests(req: &RpcRequest, contex: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        // TODO: The endpoint name matches geth's endpoint for compatibility, consider changing it in the future
        "txpool_content" => mempool::content(contex),
        "txpool_status" => mempool::status(contex),
        unknown_mempool_method => Err(RpcErr::MethodNotFound(unknown_mempool_method.to_owned())),
    }
}

/// Formats a handler result into a JSON-RPC 2.0 response.
///
/// Wraps the result in either a success response (with `result` field) or
/// an error response (with `error` field containing code and message).
///
/// # Arguments
///
/// * `id` - The request ID to include in the response (must match the request)
/// * `res` - The handler result, either success value or error
///
/// # Returns
///
/// A JSON value representing the complete JSON-RPC 2.0 response object.
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
    use crate::test_utils::default_context_with_storage;
    use ethrex_common::{
        H160,
        types::{ChainConfig, Genesis},
    };
    use ethrex_crypto::keccak::keccak_hash;
    use ethrex_storage::{EngineType, Store};
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
        let mut storage =
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
            "cancun": { "baseFeeUpdateFraction": 3338477, "max": 6, "target": 3,  },
            "prague": { "baseFeeUpdateFraction": 5007716, "max": 9, "target": 6,  },
            "osaka": { "baseFeeUpdateFraction": 5007716, "max": 9, "target": 6,  },
            "bpo1": { "baseFeeUpdateFraction": 8346193, "max": 15, "target": 10,  },
            "bpo2": { "baseFeeUpdateFraction": 11684671, "max": 21, "target": 14,  },
        });
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "enode": "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@127.0.0.1:30303",
                "enr": enr_url,
                "id": hex::encode(keccak_hash(local_p2p_node.public_key)),
                "ip": "127.0.0.1",
                "name": "ethrex/v0.1.0-test-abcd1234/x86_64-unknown-linux/rustc-v1.70.0",
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
                        "amsterdamTime": null,
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
        let mut storage =
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
        let mut storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        storage
            .set_chain_config(&example_chain_config())
            .await
            .unwrap();
        let chain_id = storage.get_chain_config().chain_id.to_string();
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
                        "P256VERIFY":"0x0000000000000000000000000000000000000100",
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
