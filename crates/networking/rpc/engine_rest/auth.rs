//! JWT bearer auth + X-Engine-Client-Version capture middleware
//! for the engine REST sub-router.

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use tracing::{debug, warn};

use crate::authentication::validate_jwt_authentication;
use crate::engine_rest::error::ProblemJson;

const X_ENGINE_CLIENT_VERSION: &str = "x-engine-client-version";
const BEARER_PREFIX: &str = "Bearer ";

/// Captured `X-Engine-Client-Version` header value, attached to request extensions.
///
/// Structured parsing is deferred until the spec finalizes; for now we keep the
/// raw string for diagnostics.
#[derive(Debug, Clone)]
pub struct EngineClientVersion {
    pub raw: String,
}

/// Tower middleware enforcing JWT bearer auth + capturing client version header.
///
/// Registered via:
/// ```ignore
/// .layer(axum::middleware::from_fn_with_state(jwt_secret, engine_auth_middleware))
/// ```
pub async fn engine_auth_middleware(
    State(secret): State<Bytes>,
    mut req: Request,
    next: Next,
) -> Response {
    let auth_value = match req.headers().get(header::AUTHORIZATION) {
        Some(v) => v,
        None => {
            return ProblemJson::unauthorized("missing Authorization header").into_response();
        }
    };

    let auth_str = match auth_value.to_str() {
        Ok(s) => s,
        Err(_) => {
            return ProblemJson::unauthorized("invalid Authorization header encoding")
                .into_response();
        }
    };

    let token = match auth_str.strip_prefix(BEARER_PREFIX) {
        Some(t) => t,
        None => {
            return ProblemJson::unauthorized("Authorization header must be Bearer")
                .into_response();
        }
    };

    if let Err(err) = validate_jwt_authentication(token, &secret) {
        debug!("engine REST auth rejected: {err:?}");
        return ProblemJson::unauthorized("JWT validation failed").into_response();
    }

    if let Some(hv) = req.headers().get(X_ENGINE_CLIENT_VERSION) {
        match hv.to_str() {
            Ok(raw) => {
                let cv = EngineClientVersion {
                    raw: raw.to_string(),
                };
                debug!(client_version = %cv.raw, "engine REST request");
                req.extensions_mut().insert(cv);
            }
            Err(_) => {
                warn!("X-Engine-Client-Version header has non-ASCII value; ignored");
            }
        }
    }

    next.run(req).await
}
