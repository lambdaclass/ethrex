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
//! Per-route body caps follow #764 §Security considerations: tight bounds for
//! small request types (forkchoice, blobs, bodies, capabilities, client/version)
//! and a generous cap for `newPayload`. Caps shadow the authrpc-wide 256 MB
//! `DefaultBodyLimit` for engine_rest routes only.

pub mod auth;
pub mod conversions;
pub mod error;
pub mod extractors;
pub mod handlers;
pub mod observe;
pub mod responses;
pub mod types;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

use crate::rpc::{ClientVersion, RpcApiContext};

/// Per-route body size caps. Bounds derived from the SSZ `MAX_*` constants in
/// `types::common`, rounded up to convenient powers of two. `newPayload` must
/// accept full execution payloads (incl. all transactions, blob commitments,
/// BAL bytes); other endpoints carry only small fixed structures.
mod body_limits {
    /// `newPayload` carries a full execution payload. Worst-case mainnet blocks
    /// are well under 10 MB; 32 MB leaves headroom for future fork bloat without
    /// inheriting the 256 MB authrpc-wide cap.
    pub const NEW_PAYLOAD: usize = 32 * 1024 * 1024;
    /// `forkchoiceUpdated` carries a ForkchoiceState (96 B) + optional
    /// PayloadAttributes (bounded by `MAX_WITHDRAWALS_PER_PAYLOAD = 16`).
    pub const FORKCHOICE: usize = 64 * 1024;
    /// `getBlobs` carries up to `MAX_BLOB_HASHES_REQUEST (128)` Bytes32 hashes
    /// (4 KB payload).
    pub const BLOBS: usize = 16 * 1024;
    /// `getPayloadBodiesByHash` carries up to `MAX_PAYLOAD_BODIES_REQUEST (32)`
    /// Bytes32 hashes (1 KB payload).
    pub const BODIES_BY_HASH: usize = 8 * 1024;
    /// `getPayloadBodiesByRange` carries `start` + `count` (16 B).
    pub const BODIES_BY_RANGE: usize = 1024;
    /// `exchangeCapabilities` carries a list of method-name strings bounded by
    /// `MAX_CAPABILITIES (64) * MAX_CAPABILITY_NAME_LENGTH (64) = 4 KB`.
    pub const CAPABILITIES: usize = 16 * 1024;
    /// `getClientVersion` carries one ClientVersionV1 (≤ ~150 B).
    pub const CLIENT_VERSION: usize = 4 * 1024;
}

/// Build the engine REST sub-router. JWT auth middleware is applied uniformly.
pub fn router(ctx: RpcApiContext) -> Router {
    let secret = ctx.node_data.jwt_secret.clone();
    let client_version: ClientVersion = ctx.node_data.client_version.clone();

    let new_payload_limit = DefaultBodyLimit::max(body_limits::NEW_PAYLOAD);
    let forkchoice_limit = DefaultBodyLimit::max(body_limits::FORKCHOICE);
    let blobs_limit = DefaultBodyLimit::max(body_limits::BLOBS);
    let bodies_by_hash_limit = DefaultBodyLimit::max(body_limits::BODIES_BY_HASH);
    let bodies_by_range_limit = DefaultBodyLimit::max(body_limits::BODIES_BY_RANGE);

    let client_router = Router::new()
        .route(
            "/engine/v1/client/version",
            post(handlers::client_version::client_version)
                .layer(DefaultBodyLimit::max(body_limits::CLIENT_VERSION)),
        )
        .with_state(client_version);

    let other_router: Router<()> = Router::new()
        // payloads (newPayload) — full execution payload, generous cap
        .route(
            "/engine/v1/payloads",
            post(handlers::payloads::new_payload_v1).layer(new_payload_limit),
        )
        .route(
            "/engine/v2/payloads",
            post(handlers::payloads::new_payload_v2).layer(new_payload_limit),
        )
        .route(
            "/engine/v3/payloads",
            post(handlers::payloads::new_payload_v3).layer(new_payload_limit),
        )
        .route(
            "/engine/v4/payloads",
            post(handlers::payloads::new_payload_v4).layer(new_payload_limit),
        )
        .route(
            "/engine/v5/payloads",
            post(handlers::payloads::new_payload_v5).layer(new_payload_limit),
        )
        // getPayload — GET, no request body
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
            post(handlers::bodies::bodies_by_hash_v1).layer(bodies_by_hash_limit),
        )
        .route(
            "/engine/v2/payloads/bodies/by-hash",
            post(handlers::bodies::bodies_by_hash_v2).layer(bodies_by_hash_limit),
        )
        .route(
            "/engine/v1/payloads/bodies/by-range",
            post(handlers::bodies::bodies_by_range_v1).layer(bodies_by_range_limit),
        )
        .route(
            "/engine/v2/payloads/bodies/by-range",
            post(handlers::bodies::bodies_by_range_v2).layer(bodies_by_range_limit),
        )
        // forkchoice
        .route(
            "/engine/v1/forkchoice",
            post(handlers::forkchoice::forkchoice_v1).layer(forkchoice_limit),
        )
        .route(
            "/engine/v2/forkchoice",
            post(handlers::forkchoice::forkchoice_v2).layer(forkchoice_limit),
        )
        .route(
            "/engine/v3/forkchoice",
            post(handlers::forkchoice::forkchoice_v3).layer(forkchoice_limit),
        )
        .route(
            "/engine/v4/forkchoice",
            post(handlers::forkchoice::forkchoice_v4).layer(forkchoice_limit),
        )
        // blobs
        .route(
            "/engine/v1/blobs",
            post(handlers::blobs::blobs_v1).layer(blobs_limit),
        )
        .route(
            "/engine/v2/blobs",
            post(handlers::blobs::blobs_v2).layer(blobs_limit),
        )
        .route(
            "/engine/v3/blobs",
            post(handlers::blobs::blobs_v3).layer(blobs_limit),
        )
        // capabilities
        .route(
            "/engine/v1/capabilities",
            post(handlers::capabilities::capabilities)
                .layer(DefaultBodyLimit::max(body_limits::CAPABILITIES)),
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
