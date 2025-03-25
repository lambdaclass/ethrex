use crate::authentication::authenticate;
use crate::context::{RpcApiContext, FILTER_DURATION};
use crate::errors::{RpcErr, RpcErrorMetadata};
use crate::eth::filter;
use crate::router::{map_authrpc_requests, map_http_requests};
use crate::rpc_types::{RpcErrorResponse, RpcRequest, RpcRequestId, RpcSuccessResponse};
use axum::extract::State;
use axum::{Json, Router};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_p2p::types::Node;
use ethrex_p2p::{sync::SyncManager, types::NodeRecord};
use ethrex_storage::Store;
use serde::Deserialize;
use serde_json::Value;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;
use tokio::sync::Mutex as TokioMutex;
use tower_http::cors::CorsLayer;
use tracing::info;

cfg_if::cfg_if! {
    if #[cfg(feature = "l2")] {
        use ethrex_common::Address;
        use secp256k1::SecretKey;
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum RpcRequestWrapper {
    Single(RpcRequest),
    Multiple(Vec<RpcRequest>),
}

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
    #[cfg(feature = "based")] gateway_eth_client: crate::clients::EthClient,
    #[cfg(feature = "based")] gateway_auth_client: crate::clients::EngineClient,
    #[cfg(feature = "l2")] valid_delegation_addresses: Vec<Address>,
    #[cfg(feature = "l2")] sponsor_pk: SecretKey,
) {
    // TODO: Refactor how filters are handled,
    // filters are used by the filters endpoints (eth_newFilter, eth_getFilterChanges, ...etc)
    let active_filters = Arc::new(Mutex::new(HashMap::new()));
    let service_context = RpcApiContext {
        storage,
        blockchain,
        jwt_secret,
        local_p2p_node,
        local_node_record,
        active_filters: active_filters.clone(),
        syncer: Arc::new(TokioMutex::new(syncer)),
        #[cfg(feature = "based")]
        gateway_eth_client,
        #[cfg(feature = "based")]
        gateway_auth_client,
        #[cfg(feature = "l2")]
        valid_delegation_addresses,
        #[cfg(feature = "l2")]
        sponsor_pk,
    };

    // Periodically clean up the active filters for the filters endpoints.
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(FILTER_DURATION);
        let filters = active_filters.clone();
        loop {
            interval.tick().await;
            tracing::info!("Running filter clean task");
            filter::clean_outdated_filters(filters.clone(), FILTER_DURATION);
            tracing::info!("Filter clean task complete");
        }
    });

    setup_and_start_servers(http_addr, authrpc_addr, service_context).await;
}

async fn setup_and_start_servers(
    http_addr: SocketAddr,
    authrpc_addr: SocketAddr,
    service_context: RpcApiContext,
) {
    // All request headers allowed.
    // All methods allowed.
    // All origins allowed.
    // All headers exposed.
    let cors = CorsLayer::permissive();

    let http_router = Router::new()
        .route("/", axum::routing::post(handle_http_request))
        .layer(cors)
        .with_state(service_context.clone());
    let http_listener = TcpListener::bind(http_addr).await.unwrap();

    let authrpc_router = Router::new()
        .route("/", axum::routing::post(handle_authrpc_request))
        .with_state(service_context);
    let authrpc_listener = TcpListener::bind(authrpc_addr).await.unwrap();

    let authrpc_server = axum::serve(authrpc_listener, authrpc_router)
        .with_graceful_shutdown(shutdown_signal())
        .into_future();
    let http_server = axum::serve(http_listener, http_router)
        .with_graceful_shutdown(shutdown_signal())
        .into_future();

    info!("Starting HTTP server at {http_addr}");
    info!("Starting Auth-RPC server at {}", authrpc_addr);

    let _ = tokio::try_join!(authrpc_server, http_server)
        .inspect_err(|e| info!("Error shutting down servers: {:?}", e));
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
}

async fn handle_http_request(
    State(service_context): State<RpcApiContext>,
    body: String,
) -> Json<Value> {
    let res = match serde_json::from_str::<RpcRequestWrapper>(&body) {
        Ok(RpcRequestWrapper::Single(request)) => {
            let res = map_http_requests(&request, service_context).await;
            rpc_response(request.id, res)
        }
        Ok(RpcRequestWrapper::Multiple(requests)) => {
            let mut responses = Vec::new();
            for req in requests {
                let res = map_http_requests(&req, service_context.clone()).await;
                responses.push(rpc_response(req.id, res));
            }
            serde_json::to_value(responses).unwrap()
        }
        Err(_) => rpc_response(
            RpcRequestId::String("".to_string()),
            Err(RpcErr::BadParams("Invalid request body".to_string())),
        ),
    };
    Json(res)
}

async fn handle_authrpc_request(
    State(service_context): State<RpcApiContext>,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    body: String,
) -> Json<Value> {
    let req: RpcRequest = match serde_json::from_str(&body) {
        Ok(req) => req,
        Err(_) => {
            return Json(rpc_response(
                RpcRequestId::String("".to_string()),
                Err(RpcErr::BadParams("Invalid request body".to_string())),
            ));
        }
    };
    match authenticate(&service_context.jwt_secret, auth_header) {
        Err(error) => Json(rpc_response(req.id, Err(error))),
        Ok(()) => {
            // Proceed with the request
            let res = map_authrpc_requests(&req, service_context).await;
            Json(rpc_response(req.id, res))
        }
    }
}

pub fn rpc_response<E>(id: RpcRequestId, res: Result<Value, E>) -> Value
where
    E: Into<RpcErrorMetadata>,
{
    match res {
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
    }
    .unwrap()
}
