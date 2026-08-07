//! REST/SSZ Engine API per execution-apis PR #793
//! (`src/engine/refactor-ssz.md`).
//!
//! All hot-path, bodies, and blobs handlers are wired, with SSZ wire types per
//! fork under `types/`. Benchmarks live in `benches/` and `tooling/engine_bench/`.

pub mod auth;
pub mod error;
pub mod extractors;
pub mod fork_header;
pub mod handlers;
pub mod responses;
pub mod types;

use axum::Router;
use axum::routing::{get, post};

use crate::rpc::{ClientVersion, RpcApiContext};

/// SSZ request/response media type for the engine REST API. Shared by the body
/// extractor, the `SszBody` responder, and the `Content-Type` validation helper.
pub(crate) const CONTENT_TYPE_OCTET_STREAM: &str = "application/octet-stream";

/// Build the engine REST sub-router. Layered with JWT auth at construction time.
/// Mounted under `/engine/v1` by the caller (see `rpc::start_api`), so the
/// effective paths are `/engine/v1/<route>` per refactor.md.
///
/// Fork-scoped routes select the fork via the `Eth-Execution-Version: <fork>`
/// request header (see [`fork_header::ExecutionVersion`]); the URL no longer
/// carries a `{fork}` segment.
///
/// Routes (relative to the `/engine/v1` mount point):
///   GET  /identity
///   GET  /capabilities
///   POST /payloads
///   GET  /payloads/{id}
///   POST /forkchoice
///   POST /bodies/hash
///   GET  /bodies
///   POST /blobs/v{1..4}
pub fn router(ctx: RpcApiContext) -> Router {
    let secret = ctx.node_data.jwt_secret.clone();
    let client_version: ClientVersion = ctx.node_data.client_version.clone();

    // /identity needs State<ClientVersion>; everything else needs no state. We
    // build two sub-routers and merge them so state types compose cleanly,
    // then apply auth middleware uniformly.
    let identity_router = Router::new()
        .route("/identity", get(handlers::identity::get_identity))
        .with_state(client_version);

    let other_router: Router<()> = Router::new()
        .route(
            "/capabilities",
            get(handlers::capabilities::get_capabilities),
        )
        .route("/payloads", post(handlers::payloads::submit_payload))
        .route("/payloads/{id}", get(handlers::payloads::get_payload))
        .route("/forkchoice", post(handlers::forkchoice::forkchoice_update))
        .route("/bodies/hash", post(handlers::bodies::bodies_by_hash))
        .route("/bodies", get(handlers::bodies::bodies_by_range))
        .route("/blobs/v1", post(handlers::blobs::blobs_v1))
        .route("/blobs/v2", post(handlers::blobs::blobs_v2))
        .route("/blobs/v3", post(handlers::blobs::blobs_v3))
        .route("/blobs/v4", post(handlers::blobs::blobs_v4))
        .with_state(ctx);

    identity_router
        .merge(other_router)
        .layer(axum::middleware::from_fn_with_state(
            secret,
            auth::engine_auth_middleware,
        ))
}
