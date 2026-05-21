//! Metrics + warn-on-error middleware for engine REST.
//!
//! Records into the same `rpc_request_duration_seconds` / `rpc_requests_total`
//! series JSON-RPC uses, with `method=engine_*` labels so dashboards combine
//! both transports. The `error_kind` label is carried in
//! `EngineErrorContext` from the error sites so it matches the JSON-RPC
//! `crate::rpc::get_error_kind` vocabulary where possible.

use axum::extract::Request;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use ethrex_metrics::rpc::{RpcOutcome, record_async_duration, record_rpc_outcome};
use std::time::Instant;
use tracing::warn;

use crate::engine_rest::error::{EngineErrorContext, error_kind_from_status};

pub async fn engine_rest_observe_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let Some(name) = jsonrpc_method_for(&method, req.uri().path()) else {
        return next.run(req).await;
    };

    let uri = req.uri().clone();

    let started = Instant::now();
    let resp = record_async_duration("engine", name, next.run(req)).await;
    let elapsed = started.elapsed();

    let status = resp.status();
    if status.is_success() {
        record_rpc_outcome("engine", name, RpcOutcome::Success);
        return resp;
    }

    let (err_msg, error_kind) = resp
        .extensions()
        .get::<EngineErrorContext>()
        .map(|c| (c.message.as_str(), c.error_kind))
        .unwrap_or(("", error_kind_from_status(status)));
    record_rpc_outcome("engine", name, RpcOutcome::Error(error_kind));
    warn!(
        method = name,
        http_method = %method,
        path = uri.path(),
        status = %status,
        duration_ms = elapsed.as_millis() as u64,
        err_msg,
        transport = "ssz",
        "engine REST non-2xx response (CL will fall back to JSON-RPC)",
    );
    resp
}

/// Map (HTTP method, path) → JSON-RPC method name. `None` skips instrumentation.
fn jsonrpc_method_for(http_method: &Method, path: &str) -> Option<&'static str> {
    if !path.starts_with("/engine/v") {
        return None;
    }
    let method = http_method.as_str();

    // `GET /engine/v{N}/payloads/{id}` — match on the prefix before the id segment.
    if method == "GET"
        && let Some((prefix, _id)) = path.rsplit_once("/payloads/")
    {
        return Some(match prefix {
            "/engine/v1" => "engine_getPayloadV1",
            "/engine/v2" => "engine_getPayloadV2",
            "/engine/v3" => "engine_getPayloadV3",
            "/engine/v4" => "engine_getPayloadV4",
            "/engine/v5" => "engine_getPayloadV5",
            "/engine/v6" => "engine_getPayloadV6",
            _ => return None,
        });
    }

    Some(match (method, path) {
        ("POST", "/engine/v1/payloads") => "engine_newPayloadV1",
        ("POST", "/engine/v2/payloads") => "engine_newPayloadV2",
        ("POST", "/engine/v3/payloads") => "engine_newPayloadV3",
        ("POST", "/engine/v4/payloads") => "engine_newPayloadV4",
        ("POST", "/engine/v5/payloads") => "engine_newPayloadV5",
        ("POST", "/engine/v1/payloads/bodies/by-hash") => "engine_getPayloadBodiesByHashV1",
        ("POST", "/engine/v2/payloads/bodies/by-hash") => "engine_getPayloadBodiesByHashV2",
        ("POST", "/engine/v1/payloads/bodies/by-range") => "engine_getPayloadBodiesByRangeV1",
        ("POST", "/engine/v2/payloads/bodies/by-range") => "engine_getPayloadBodiesByRangeV2",
        ("POST", "/engine/v1/forkchoice") => "engine_forkchoiceUpdatedV1",
        ("POST", "/engine/v2/forkchoice") => "engine_forkchoiceUpdatedV2",
        ("POST", "/engine/v3/forkchoice") => "engine_forkchoiceUpdatedV3",
        ("POST", "/engine/v4/forkchoice") => "engine_forkchoiceUpdatedV4",
        ("POST", "/engine/v1/blobs") => "engine_getBlobsV1",
        ("POST", "/engine/v2/blobs") => "engine_getBlobsV2",
        ("POST", "/engine/v3/blobs") => "engine_getBlobsV3",
        ("POST", "/engine/v1/client/version") => "engine_getClientVersionV1",
        ("POST", "/engine/v1/capabilities") => "engine_exchangeCapabilities",
        _ => return None,
    })
}
