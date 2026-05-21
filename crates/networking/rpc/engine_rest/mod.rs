//! Engine REST/SSZ transport per execution-apis PR #764.
//!
//! Binary SSZ endpoints under `/engine/v{N}/...` on the authrpc port. JSON-RPC
//! stays the default; supported endpoints are advertised via
//! `engine_exchangeCapabilities` as strings like `"POST /engine/v5/payloads"`.
//!
//! Endpoints (versions correspond to `engine_*V{N}` JSON-RPC method versions):
//!   POST /engine/v{1..5}/payloads                       newPayload
//!   GET  /engine/v{1..6}/payloads/{payload_id}          getPayload
//!   POST /engine/v{1,2}/payloads/bodies/by-hash         getPayloadBodiesByHash
//!   POST /engine/v{1,2}/payloads/bodies/by-range        getPayloadBodiesByRange
//!   POST /engine/v{1..4}/forkchoice                     forkchoiceUpdated
//!   POST /engine/v{1..3}/blobs                          getBlobs
//!   POST /engine/v1/client/version                      getClientVersion
//!   POST /engine/v1/capabilities                        exchangeCapabilities
//!
//! Per-endpoint Content-Length caps from #764 §Security considerations are
//! not enforced; the authrpc router's global 256 MB `DefaultBodyLimit` covers
//! both transports.

pub mod auth;
pub mod conversions;
pub mod error;
pub mod extractors;
pub mod handlers;
pub mod observe;
pub mod responses;
pub mod types;

use axum::Router;
use axum::routing::{get, post};

use crate::rpc::{ClientVersion, RpcApiContext};

/// Build the engine REST sub-router. JWT auth middleware is applied uniformly.
pub fn router(ctx: RpcApiContext) -> Router {
    let secret = ctx.node_data.jwt_secret.clone();
    let client_version: ClientVersion = ctx.node_data.client_version.clone();

    let client_router = Router::new()
        .route(
            "/engine/v1/client/version",
            post(handlers::client_version::client_version),
        )
        .with_state(client_version);

    let other_router: Router<()> = Router::new()
        // payloads
        .route(
            "/engine/v1/payloads",
            post(handlers::payloads::new_payload_v1),
        )
        .route(
            "/engine/v2/payloads",
            post(handlers::payloads::new_payload_v2),
        )
        .route(
            "/engine/v3/payloads",
            post(handlers::payloads::new_payload_v3),
        )
        .route(
            "/engine/v4/payloads",
            post(handlers::payloads::new_payload_v4),
        )
        .route(
            "/engine/v5/payloads",
            post(handlers::payloads::new_payload_v5),
        )
        .route(
            "/engine/v1/payloads/{payload_id}",
            get(handlers::payloads::get_payload_v1),
        )
        .route(
            "/engine/v2/payloads/{payload_id}",
            get(handlers::payloads::get_payload_v2),
        )
        .route(
            "/engine/v3/payloads/{payload_id}",
            get(handlers::payloads::get_payload_v3),
        )
        .route(
            "/engine/v4/payloads/{payload_id}",
            get(handlers::payloads::get_payload_v4),
        )
        .route(
            "/engine/v5/payloads/{payload_id}",
            get(handlers::payloads::get_payload_v5),
        )
        .route(
            "/engine/v6/payloads/{payload_id}",
            get(handlers::payloads::get_payload_v6),
        )
        // bodies
        .route(
            "/engine/v1/payloads/bodies/by-hash",
            post(handlers::bodies::bodies_by_hash_v1),
        )
        .route(
            "/engine/v2/payloads/bodies/by-hash",
            post(handlers::bodies::bodies_by_hash_v2),
        )
        .route(
            "/engine/v1/payloads/bodies/by-range",
            post(handlers::bodies::bodies_by_range_v1),
        )
        .route(
            "/engine/v2/payloads/bodies/by-range",
            post(handlers::bodies::bodies_by_range_v2),
        )
        // forkchoice
        .route(
            "/engine/v1/forkchoice",
            post(handlers::forkchoice::forkchoice_v1),
        )
        .route(
            "/engine/v2/forkchoice",
            post(handlers::forkchoice::forkchoice_v2),
        )
        .route(
            "/engine/v3/forkchoice",
            post(handlers::forkchoice::forkchoice_v3),
        )
        .route(
            "/engine/v4/forkchoice",
            post(handlers::forkchoice::forkchoice_v4),
        )
        // blobs
        .route("/engine/v1/blobs", post(handlers::blobs::blobs_v1))
        .route("/engine/v2/blobs", post(handlers::blobs::blobs_v2))
        .route("/engine/v3/blobs", post(handlers::blobs::blobs_v3))
        // capabilities
        .route(
            "/engine/v1/capabilities",
            post(handlers::capabilities::capabilities),
        )
        .with_state(ctx);

    client_router
        .merge(other_router)
        // Observe runs inside auth so unauthenticated requests don't pollute counters.
        .layer(axum::middleware::from_fn(
            observe::engine_rest_observe_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            secret,
            auth::engine_auth_middleware,
        ))
}

/// SSZ-REST endpoints advertised via `engine_exchangeCapabilities`, formatted
/// as `"METHOD /path"`.
pub const SSZ_REST_CAPABILITIES: &[&str] = &[
    "POST /engine/v1/payloads",
    "POST /engine/v2/payloads",
    "POST /engine/v3/payloads",
    "POST /engine/v4/payloads",
    "POST /engine/v5/payloads",
    "GET /engine/v1/payloads/{payload_id}",
    "GET /engine/v2/payloads/{payload_id}",
    "GET /engine/v3/payloads/{payload_id}",
    "GET /engine/v4/payloads/{payload_id}",
    "GET /engine/v5/payloads/{payload_id}",
    "GET /engine/v6/payloads/{payload_id}",
    "POST /engine/v1/payloads/bodies/by-hash",
    "POST /engine/v2/payloads/bodies/by-hash",
    "POST /engine/v1/payloads/bodies/by-range",
    "POST /engine/v2/payloads/bodies/by-range",
    "POST /engine/v1/forkchoice",
    "POST /engine/v2/forkchoice",
    "POST /engine/v3/forkchoice",
    "POST /engine/v4/forkchoice",
    "POST /engine/v1/blobs",
    "POST /engine/v2/blobs",
    "POST /engine/v3/blobs",
    "POST /engine/v1/client/version",
    "POST /engine/v1/capabilities",
];
