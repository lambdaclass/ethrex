//! Engine API implementation using raw Hyper
//!
//! This module provides a raw Hyper-based implementation of the Engine API
//! for benchmarking purposes. This represents the lowest overhead possible
//! since Axum is built on top of Hyper.

use crate::authentication::validate_jwt_authentication;
use crate::engine::blobs::{BlobsV1Request, BlobsV2Request, BlobsV3Request};
use crate::engine::client_version::GetClientVersionV1Request;
use crate::engine::fork_choice::{ForkChoiceUpdatedV1, ForkChoiceUpdatedV2, ForkChoiceUpdatedV3};
use crate::engine::payload::{
    GetPayloadBodiesByHashV1Request, GetPayloadBodiesByRangeV1Request, GetPayloadV1Request,
    GetPayloadV2Request, GetPayloadV3Request, GetPayloadV4Request, GetPayloadV5Request,
    NewPayloadV1Request, NewPayloadV2Request, NewPayloadV3Request, NewPayloadV4Request,
};
use crate::engine::ExchangeCapabilitiesRequest;
use crate::engine::exchange_transition_config::ExchangeTransitionConfigV1Req;
use crate::eth::block::BlockNumberRequest;
use crate::eth::client::{ChainId, Syncing};
use crate::rpc::{RpcApiContext, RpcHandler};
use crate::utils::RpcErr;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{error, info};

/// JSON-RPC 2.0 response
#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: Value,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            result: Some(result),
            error: None,
            id,
        }
    }

    fn error(id: Value, code: i32, message: String, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data,
            }),
            id,
        }
    }
}

/// Convert RpcErr to JSON-RPC error response
fn rpc_err_to_response(id: Value, err: RpcErr) -> JsonRpcResponse {
    match err {
        RpcErr::MethodNotFound(msg) => JsonRpcResponse::error(id, -32601, msg, None),
        RpcErr::BadParams(msg) => JsonRpcResponse::error(id, -32000, msg, None),
        RpcErr::MissingParam(msg) => {
            JsonRpcResponse::error(id, -32000, format!("Missing param: {msg}"), None)
        }
        RpcErr::WrongParam(msg) => {
            JsonRpcResponse::error(id, -32602, format!("Wrong param: {msg}"), None)
        }
        RpcErr::BadHexFormat(idx) => {
            JsonRpcResponse::error(id, -32602, format!("Bad hex format at param {idx}"), None)
        }
        RpcErr::Internal(msg) => JsonRpcResponse::error(id, -32603, msg, None),
        RpcErr::Vm(msg) => JsonRpcResponse::error(id, -32603, msg, None),
        RpcErr::Revert { data } => {
            JsonRpcResponse::error(id, 3, format!("execution reverted: {data}"), Some(data.into()))
        }
        RpcErr::Halt { reason, gas_used } => JsonRpcResponse::error(
            id,
            3,
            reason,
            Some(format!("gas_used: {gas_used}").into()),
        ),
        RpcErr::AuthenticationError(err) => {
            JsonRpcResponse::error(id, -32000, format!("Auth error: {err:?}"), None)
        }
        RpcErr::InvalidForkChoiceState(msg) => JsonRpcResponse::error(id, -38002, msg, None),
        RpcErr::InvalidPayloadAttributes(msg) => JsonRpcResponse::error(id, -38003, msg, None),
        RpcErr::UnknownPayload(msg) => JsonRpcResponse::error(id, -38001, msg, None),
        RpcErr::TooLargeRequest => {
            JsonRpcResponse::error(id, -38004, "Request too large".to_string(), None)
        }
        RpcErr::UnsuportedFork(msg) => JsonRpcResponse::error(id, -38005, msg, None),
    }
}

/// Minimal JSON-RPC request parsing
#[derive(Deserialize)]
struct MinimalRpcRequest {
    method: String,
    params: Option<Vec<Value>>,
    id: Value,
}

/// Handle a single JSON-RPC request and dispatch to the appropriate handler
async fn handle_rpc_method(
    method: &str,
    params: Option<Vec<Value>>,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    match method {
        // Engine API methods
        "engine_exchangeCapabilities" => {
            let handler = ExchangeCapabilitiesRequest::parse(&params)?;
            handler.handle(context).await
        }
        "engine_forkchoiceUpdatedV1" => {
            let handler = ForkChoiceUpdatedV1::parse(&params)?;
            handler.handle(context).await
        }
        "engine_forkchoiceUpdatedV2" => {
            let handler = ForkChoiceUpdatedV2::parse(&params)?;
            handler.handle(context).await
        }
        "engine_forkchoiceUpdatedV3" => {
            let handler = ForkChoiceUpdatedV3::parse(&params)?;
            handler.handle(context).await
        }
        "engine_newPayloadV1" => {
            let handler = NewPayloadV1Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_newPayloadV2" => {
            let handler = NewPayloadV2Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_newPayloadV3" => {
            let handler = NewPayloadV3Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_newPayloadV4" => {
            let handler = NewPayloadV4Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getPayloadV1" => {
            let handler = GetPayloadV1Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getPayloadV2" => {
            let handler = GetPayloadV2Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getPayloadV3" => {
            let handler = GetPayloadV3Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getPayloadV4" => {
            let handler = GetPayloadV4Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getPayloadV5" => {
            let handler = GetPayloadV5Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getPayloadBodiesByHashV1" => {
            let handler = GetPayloadBodiesByHashV1Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getPayloadBodiesByRangeV1" => {
            let handler = GetPayloadBodiesByRangeV1Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_exchangeTransitionConfigurationV1" => {
            let handler = ExchangeTransitionConfigV1Req::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getBlobsV1" => {
            let handler = BlobsV1Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getBlobsV2" => {
            let handler = BlobsV2Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getBlobsV3" => {
            let handler = BlobsV3Request::parse(&params)?;
            handler.handle(context).await
        }
        "engine_getClientVersionV1" => {
            let handler = GetClientVersionV1Request::parse(&params)?;
            handler.handle(context).await
        }
        // Eth methods available on auth-rpc
        "eth_blockNumber" => {
            let handler = BlockNumberRequest::parse(&params)?;
            handler.handle(context).await
        }
        "eth_chainId" => {
            let handler = ChainId::parse(&params)?;
            handler.handle(context).await
        }
        "eth_syncing" => {
            let handler = Syncing::parse(&params)?;
            handler.handle(context).await
        }
        _ => Err(RpcErr::MethodNotFound(method.to_string())),
    }
}

/// Handle an incoming HTTP request
async fn handle_request(
    req: Request<Incoming>,
    context: Arc<RpcApiContext>,
    jwt_secret: Bytes,
    timer_sender: watch::Sender<()>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Send heartbeat
    let _ = timer_sender.send(());

    // Only accept POST requests
    if req.method() != Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Full::new(Bytes::from("Method not allowed")))
            .expect("valid response"));
    }

    // Validate JWT
    let auth_header = req
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(auth) if auth.starts_with("Bearer ") => {
            let token = &auth[7..];
            if validate_jwt_authentication(token, &jwt_secret).is_err() {
                let response = JsonRpcResponse::error(
                    Value::Null,
                    -32000,
                    "Invalid JWT".to_string(),
                    None,
                );
                let body = serde_json::to_vec(&response).unwrap_or_default();
                return Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(body)))
                    .expect("valid response"));
            }
        }
        _ => {
            let response = JsonRpcResponse::error(
                Value::Null,
                -32000,
                "Missing JWT".to_string(),
                None,
            );
            let body = serde_json::to_vec(&response).unwrap_or_default();
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(body)))
                .expect("valid response"));
        }
    }

    // Read body with 256MB limit
    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            let response = JsonRpcResponse::error(
                Value::Null,
                -32700,
                "Parse error".to_string(),
                None,
            );
            let body = serde_json::to_vec(&response).unwrap_or_default();
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(body)))
                .expect("valid response"));
        }
    };

    // Check body size (256MB limit)
    if body_bytes.len() > 256 * 1024 * 1024 {
        let response = JsonRpcResponse::error(
            Value::Null,
            -38004,
            "Request too large".to_string(),
            None,
        );
        let body = serde_json::to_vec(&response).unwrap_or_default();
        return Ok(Response::builder()
            .status(StatusCode::PAYLOAD_TOO_LARGE)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .expect("valid response"));
    }

    // Parse JSON-RPC request
    let rpc_request: MinimalRpcRequest = match serde_json::from_slice(&body_bytes) {
        Ok(req) => req,
        Err(_) => {
            let response = JsonRpcResponse::error(
                Value::Null,
                -32700,
                "Parse error".to_string(),
                None,
            );
            let body = serde_json::to_vec(&response).unwrap_or_default();
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(body)))
                .expect("valid response"));
        }
    };

    // Handle the RPC method
    let response = match handle_rpc_method(
        &rpc_request.method,
        rpc_request.params,
        (*context).clone(),
    )
    .await
    {
        Ok(result) => JsonRpcResponse::success(rpc_request.id, result),
        Err(err) => rpc_err_to_response(rpc_request.id, err),
    };

    let body = serde_json::to_vec(&response).unwrap_or_default();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .expect("valid response"))
}

/// Start the raw Hyper-based Engine API server
pub async fn start_authrpc_server(
    addr: SocketAddr,
    context: RpcApiContext,
    timer_sender: watch::Sender<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let jwt_secret = context.node_data.jwt_secret.clone();
    let context = Arc::new(context);

    let listener = TcpListener::bind(addr).await?;
    info!("Starting Hyper Auth-RPC server at {addr}");

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        let context = context.clone();
        let jwt_secret = jwt_secret.clone();
        let timer_sender = timer_sender.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req| {
                let context = context.clone();
                let jwt_secret = jwt_secret.clone();
                let timer_sender = timer_sender.clone();
                async move { handle_request(req, context, jwt_secret, timer_sender).await }
            });

            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                error!("Error serving connection: {err:?}");
            }
        });
    }
}
