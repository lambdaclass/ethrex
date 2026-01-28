//! Engine API implementation using jsonrpsee
//!
//! This module provides a jsonrpsee-based implementation of the Engine API
//! for benchmarking purposes against the Axum-based implementation.

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
use crate::utils::RpcRequest;
use bytes::Bytes;
use jsonrpsee::server::middleware::rpc::{RpcServiceBuilder, RpcServiceT};
use jsonrpsee::server::{ServerBuilder, ServerHandle};
use jsonrpsee::types::{ErrorObject, Request};
use jsonrpsee::{MethodResponse, RpcModule};
use serde_json::Value;
use std::net::SocketAddr;
use tokio::sync::watch;
use tower::Layer;
use tracing::info;

/// Error codes for Engine API
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;
const UNKNOWN_PAYLOAD: i32 = -38001;
const INVALID_FORKCHOICE_STATE: i32 = -38002;
const INVALID_PAYLOAD_ATTRIBUTES: i32 = -38003;
const TOO_LARGE_REQUEST: i32 = -38004;
const UNSUPPORTED_FORK: i32 = -38005;

/// Convert our RpcErr to jsonrpsee ErrorObject
fn to_error_object(err: crate::utils::RpcErr) -> ErrorObject<'static> {
    use crate::utils::RpcErr;
    match err {
        RpcErr::MethodNotFound(msg) => ErrorObject::owned(METHOD_NOT_FOUND, msg, None::<()>),
        RpcErr::BadParams(msg) => ErrorObject::owned(INVALID_PARAMS, msg, None::<()>),
        RpcErr::MissingParam(msg) => {
            ErrorObject::owned(INVALID_PARAMS, format!("Missing param: {msg}"), None::<()>)
        }
        RpcErr::WrongParam(msg) => {
            ErrorObject::owned(INVALID_PARAMS, format!("Wrong param: {msg}"), None::<()>)
        }
        RpcErr::BadHexFormat(idx) => ErrorObject::owned(
            INVALID_PARAMS,
            format!("Bad hex format at param {idx}"),
            None::<()>,
        ),
        RpcErr::Internal(msg) => ErrorObject::owned(INTERNAL_ERROR, msg, None::<()>),
        RpcErr::Vm(err) => ErrorObject::owned(INTERNAL_ERROR, err.to_string(), None::<()>),
        RpcErr::Revert { data } => {
            ErrorObject::owned(3, format!("execution reverted: {data}"), Some(data))
        }
        RpcErr::Halt { reason, gas_used } => {
            ErrorObject::owned(3, reason, Some(format!("gas_used: {gas_used}")))
        }
        RpcErr::AuthenticationError(err) => {
            ErrorObject::owned(INVALID_REQUEST, format!("Auth error: {err:?}"), None::<()>)
        }
        RpcErr::InvalidForkChoiceState(msg) => {
            ErrorObject::owned(INVALID_FORKCHOICE_STATE, msg, None::<()>)
        }
        RpcErr::InvalidPayloadAttributes(msg) => {
            ErrorObject::owned(INVALID_PAYLOAD_ATTRIBUTES, msg, None::<()>)
        }
        RpcErr::UnknownPayload(msg) => ErrorObject::owned(UNKNOWN_PAYLOAD, msg, None::<()>),
        RpcErr::TooLargeRequest => {
            ErrorObject::owned(TOO_LARGE_REQUEST, "Request too large", None::<()>)
        }
        RpcErr::UnsuportedFork(msg) => ErrorObject::owned(UNSUPPORTED_FORK, msg, None::<()>),
    }
}

/// JWT Authentication middleware for jsonrpsee
#[derive(Clone)]
pub struct JwtAuthLayer {
    jwt_secret: Bytes,
}

impl JwtAuthLayer {
    pub fn new(jwt_secret: Bytes) -> Self {
        Self { jwt_secret }
    }
}

impl<S> Layer<S> for JwtAuthLayer {
    type Service = JwtAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        JwtAuthService { inner }
    }
}

#[derive(Clone)]
pub struct JwtAuthService<S> {
    inner: S,
    // JWT validation is done at the HTTP layer via JwtValidationLayer
}

impl<'a, S> RpcServiceT<'a> for JwtAuthService<S>
where
    S: RpcServiceT<'a> + Send + Sync + Clone + 'static,
{
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = MethodResponse> + Send + 'a>,
    >;

    fn call(&self, req: Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}

/// Helper macro to call an RpcHandler and return jsonrpsee result
macro_rules! call_handler {
    ($handler:ty, $params:expr, $ctx:expr) => {{
        let rpc_request = RpcRequest {
            method: String::new(),
            params: $params,
            ..Default::default()
        };
        // ctx is Arc<RpcApiContext>, we need to clone the inner RpcApiContext
        let context: RpcApiContext = $ctx.as_ref().clone();
        match <$handler>::parse(&rpc_request.params) {
            Ok(handler) => match handler.handle(context).await {
                Ok(value) => Ok(value),
                Err(err) => Err(to_error_object(err)),
            },
            Err(err) => Err(to_error_object(err)),
        }
    }};
}

/// Register all Engine API methods on an RpcModule
fn register_engine_methods(
    module: &mut RpcModule<RpcApiContext>,
) -> Result<(), jsonrpsee::core::RegisterMethodError> {
    // engine_exchangeCapabilities
    module.register_async_method("engine_exchangeCapabilities", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(ExchangeCapabilitiesRequest, params, ctx)
    })?;

    // engine_forkchoiceUpdatedV1
    module.register_async_method("engine_forkchoiceUpdatedV1", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(ForkChoiceUpdatedV1, params, ctx)
    })?;

    // engine_forkchoiceUpdatedV2
    module.register_async_method("engine_forkchoiceUpdatedV2", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(ForkChoiceUpdatedV2, params, ctx)
    })?;

    // engine_forkchoiceUpdatedV3
    module.register_async_method("engine_forkchoiceUpdatedV3", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(ForkChoiceUpdatedV3, params, ctx)
    })?;

    // engine_newPayloadV1
    module.register_async_method("engine_newPayloadV1", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(NewPayloadV1Request, params, ctx)
    })?;

    // engine_newPayloadV2
    module.register_async_method("engine_newPayloadV2", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(NewPayloadV2Request, params, ctx)
    })?;

    // engine_newPayloadV3
    module.register_async_method("engine_newPayloadV3", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(NewPayloadV3Request, params, ctx)
    })?;

    // engine_newPayloadV4
    module.register_async_method("engine_newPayloadV4", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(NewPayloadV4Request, params, ctx)
    })?;

    // engine_getPayloadV1
    module.register_async_method("engine_getPayloadV1", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(GetPayloadV1Request, params, ctx)
    })?;

    // engine_getPayloadV2
    module.register_async_method("engine_getPayloadV2", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(GetPayloadV2Request, params, ctx)
    })?;

    // engine_getPayloadV3
    module.register_async_method("engine_getPayloadV3", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(GetPayloadV3Request, params, ctx)
    })?;

    // engine_getPayloadV4
    module.register_async_method("engine_getPayloadV4", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(GetPayloadV4Request, params, ctx)
    })?;

    // engine_getPayloadV5
    module.register_async_method("engine_getPayloadV5", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(GetPayloadV5Request, params, ctx)
    })?;

    // engine_getPayloadBodiesByHashV1
    module.register_async_method(
        "engine_getPayloadBodiesByHashV1",
        |params, ctx, _| async move {
            let params: Option<Vec<Value>> = params.parse().ok();
            call_handler!(GetPayloadBodiesByHashV1Request, params, ctx)
        },
    )?;

    // engine_getPayloadBodiesByRangeV1
    module.register_async_method(
        "engine_getPayloadBodiesByRangeV1",
        |params, ctx, _| async move {
            let params: Option<Vec<Value>> = params.parse().ok();
            call_handler!(
                GetPayloadBodiesByRangeV1Request,
                params,
                ctx
            )
        },
    )?;

    // engine_exchangeTransitionConfigurationV1
    module.register_async_method(
        "engine_exchangeTransitionConfigurationV1",
        |params, ctx, _| async move {
            let params: Option<Vec<Value>> = params.parse().ok();
            call_handler!(ExchangeTransitionConfigV1Req, params, ctx)
        },
    )?;

    // engine_getBlobsV1
    module.register_async_method("engine_getBlobsV1", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(BlobsV1Request, params, ctx)
    })?;

    // engine_getBlobsV2
    module.register_async_method("engine_getBlobsV2", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(BlobsV2Request, params, ctx)
    })?;

    // engine_getBlobsV3
    module.register_async_method("engine_getBlobsV3", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(BlobsV3Request, params, ctx)
    })?;

    // engine_getClientVersionV1
    module.register_async_method("engine_getClientVersionV1", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(GetClientVersionV1Request, params, ctx)
    })?;

    Ok(())
}

/// Register eth_* methods that are also available on the auth-rpc endpoint
fn register_eth_methods(
    module: &mut RpcModule<RpcApiContext>,
) -> Result<(), jsonrpsee::core::RegisterMethodError> {
    // eth_blockNumber
    module.register_async_method("eth_blockNumber", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(BlockNumberRequest, params, ctx)
    })?;

    // eth_chainId
    module.register_async_method("eth_chainId", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(ChainId, params, ctx)
    })?;

    // eth_syncing
    module.register_async_method("eth_syncing", |params, ctx, _| async move {
        let params: Option<Vec<Value>> = params.parse().ok();
        call_handler!(Syncing, params, ctx)
    })?;

    Ok(())
}

/// Start the jsonrpsee-based Engine API server
///
/// This replaces the Axum-based authrpc server with jsonrpsee.
pub async fn start_authrpc_server(
    addr: SocketAddr,
    context: RpcApiContext,
    timer_sender: watch::Sender<()>,
) -> Result<ServerHandle, Box<dyn std::error::Error + Send + Sync>> {
    let jwt_secret = context.node_data.jwt_secret.clone();

    // Build the RPC module with all engine methods
    // Store RpcApiContext directly (jsonrpsee wraps it in Arc internally)
    let mut module = RpcModule::new(context);
    register_engine_methods(&mut module)?;
    register_eth_methods(&mut module)?;

    // Create middleware that validates JWT and sends heartbeat
    let rpc_middleware = RpcServiceBuilder::new().layer_fn(move |service| {
        let timer_sender = timer_sender.clone();
        HeartbeatService {
            inner: service,
            timer_sender,
        }
    });

    // Build server with custom HTTP middleware for JWT validation
    let server = ServerBuilder::default()
        .set_rpc_middleware(rpc_middleware)
        .set_http_middleware(
            tower::ServiceBuilder::new().layer(JwtValidationLayer::new(jwt_secret)),
        )
        .max_request_body_size(256 * 1024 * 1024) // 256MB for engine payloads
        .build(addr)
        .await?;

    info!("Starting jsonrpsee Auth-RPC server at {addr}");

    let handle = server.start(module);
    Ok(handle)
}

/// Heartbeat service that notifies on each RPC call
#[derive(Clone)]
struct HeartbeatService<S> {
    inner: S,
    timer_sender: watch::Sender<()>,
}

impl<'a, S> RpcServiceT<'a> for HeartbeatService<S>
where
    S: RpcServiceT<'a> + Send + Sync + Clone + 'static,
{
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = MethodResponse> + Send + 'a>,
    >;

    fn call(&self, req: Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let timer_sender = self.timer_sender.clone();

        Box::pin(async move {
            // Send heartbeat
            let _ = timer_sender.send(());
            inner.call(req).await
        })
    }
}

/// HTTP Layer for JWT validation
#[derive(Clone)]
pub struct JwtValidationLayer {
    jwt_secret: Bytes,
}

impl JwtValidationLayer {
    pub fn new(jwt_secret: Bytes) -> Self {
        Self { jwt_secret }
    }
}

impl<S> tower::Layer<S> for JwtValidationLayer {
    type Service = JwtValidationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        JwtValidationService {
            inner,
            jwt_secret: self.jwt_secret.clone(),
        }
    }
}

#[derive(Clone)]
pub struct JwtValidationService<S> {
    inner: S,
    jwt_secret: Bytes,
}

impl<S, B> tower::Service<http::Request<B>> for JwtValidationService<S>
where
    S: tower::Service<http::Request<B>, Response = http::Response<jsonrpsee::server::HttpBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        let jwt_secret = self.jwt_secret.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // Extract Authorization header
            let auth_header = req
                .headers()
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok());

            match auth_header {
                Some(auth) if auth.starts_with("Bearer ") => {
                    let token = &auth[7..];
                    match validate_jwt_authentication(token, &jwt_secret) {
                        Ok(()) => inner.call(req).await,
                        Err(_) => {
                            let response = http::Response::builder()
                                .status(http::StatusCode::UNAUTHORIZED)
                                .body(jsonrpsee::server::HttpBody::from(
                                    r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Invalid JWT"},"id":null}"#,
                                ))
                                .expect("valid response");
                            Ok(response)
                        }
                    }
                }
                _ => {
                    let response = http::Response::builder()
                        .status(http::StatusCode::UNAUTHORIZED)
                        .body(jsonrpsee::server::HttpBody::from(
                            r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Missing JWT"},"id":null}"#,
                        ))
                        .expect("valid response");
                    Ok(response)
                }
            }
        })
    }
}
