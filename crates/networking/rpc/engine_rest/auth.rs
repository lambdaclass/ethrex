//! JWT bearer auth middleware shared with JSON-RPC.

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use bytes::Bytes;

use crate::authentication::validate_jwt_authentication;
use crate::engine_rest::error::error_response;

pub async fn engine_auth_middleware(
    State(secret): State<Bytes>,
    request: Request,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| HeaderValue::to_str(v).ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let Some(token) = token else {
        return error_response(StatusCode::UNAUTHORIZED, "missing bearer token");
    };

    if validate_jwt_authentication(token, &secret).is_err() {
        return error_response(StatusCode::UNAUTHORIZED, "authentication failed");
    }

    next.run(request).await
}
