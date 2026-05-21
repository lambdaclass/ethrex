//! Engine REST error responses with text/plain body.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::utils::RpcErr;

/// Carries the error message and the metric `error_kind` label into
/// `Response::extensions` so the observe middleware can log/record without
/// re-buffering the body or re-deriving the label from the HTTP status.
#[derive(Clone, Debug)]
pub struct EngineErrorContext {
    pub message: String,
    pub error_kind: &'static str,
}

/// Static `error_kind` label for the metric counter when only the HTTP
/// status code is known (no source `RpcErr`). Kept aligned with
/// `crate::rpc::get_error_kind` where the categories overlap.
pub(crate) fn error_kind_from_status(status: StatusCode) -> &'static str {
    match status.as_u16() {
        400 => "BadRequest",
        401 => "Unauthorized",
        404 => "NotFound",
        409 => "InvalidForkChoiceState",
        413 => "TooLargeRequest",
        422 => "InvalidPayloadAttributes",
        500 => "Internal",
        _ => "Other",
    }
}

pub fn error_response(status: StatusCode, msg: &str) -> Response {
    error_response_with_kind(status, msg, error_kind_from_status(status))
}

fn error_response_with_kind(status: StatusCode, msg: &str, error_kind: &'static str) -> Response {
    let message = msg.to_string();
    let mut resp = (status, message.clone()).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp.extensions_mut().insert(EngineErrorContext {
        message,
        error_kind,
    });
    resp
}

/// Convenience builders for the error categories listed in the spec.
pub struct EngineError;

impl EngineError {
    pub fn bad_request(msg: &str) -> Response {
        error_response(StatusCode::BAD_REQUEST, msg)
    }

    pub fn unauthorized(msg: &str) -> Response {
        error_response(StatusCode::UNAUTHORIZED, msg)
    }

    pub fn not_found(msg: &str) -> Response {
        error_response(StatusCode::NOT_FOUND, msg)
    }

    pub fn conflict(msg: &str) -> Response {
        error_response(StatusCode::CONFLICT, msg)
    }

    pub fn payload_too_large(msg: &str) -> Response {
        error_response(StatusCode::PAYLOAD_TOO_LARGE, msg)
    }

    pub fn unprocessable(msg: &str) -> Response {
        error_response(StatusCode::UNPROCESSABLE_ENTITY, msg)
    }

    pub fn internal(msg: &str) -> Response {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

/// Small (status + message) error returned from engine-REST helpers; converts
/// to an HTTP `Response` at the handler boundary via `From`/`IntoResponse`.
/// `error_kind` is the label used by the metric counter; it defaults from
/// the status code but can be overridden (e.g. by `From<RpcErr>`) to match
/// the JSON-RPC vocabulary.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct EngineRestError {
    pub status: StatusCode,
    pub message: String,
    pub error_kind: &'static str,
}

impl EngineRestError {
    pub fn new(status: StatusCode, msg: impl Into<String>) -> Self {
        Self {
            status,
            message: msg.into(),
            error_kind: error_kind_from_status(status),
        }
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, msg)
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, msg)
    }
    pub fn payload_too_large(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, msg)
    }
    pub fn unprocessable(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, msg)
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

impl From<EngineRestError> for Response {
    fn from(e: EngineRestError) -> Response {
        error_response_with_kind(e.status, &e.message, e.error_kind)
    }
}

impl IntoResponse for EngineRestError {
    fn into_response(self) -> Response {
        self.into()
    }
}

/// Alias kept so `conversions.rs` reads naturally.
pub type ConversionError = EngineRestError;

impl From<RpcErr> for EngineRestError {
    fn from(err: RpcErr) -> Self {
        let error_kind = crate::rpc::get_error_kind(&err);
        let (status, message) = match err {
            RpcErr::UnsupportedFork(m) | RpcErr::InvalidPayloadAttributes(m) => {
                (StatusCode::UNPROCESSABLE_ENTITY, m)
            }
            RpcErr::InvalidForkChoiceState(m) | RpcErr::TooDeepReorg(m) => {
                (StatusCode::CONFLICT, m)
            }
            RpcErr::UnknownPayload(m) => (StatusCode::NOT_FOUND, m),
            RpcErr::TooLargeRequest => (StatusCode::PAYLOAD_TOO_LARGE, "request too large".into()),
            RpcErr::AuthenticationError(_) => {
                (StatusCode::UNAUTHORIZED, "authentication failed".into())
            }
            RpcErr::WrongParam(_)
            | RpcErr::BadParams(_)
            | RpcErr::MissingParam(_)
            | RpcErr::BadHexFormat(_) => (StatusCode::BAD_REQUEST, err.to_string()),
            other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        };
        Self {
            status,
            message,
            error_kind,
        }
    }
}

/// Map an `RpcErr` to an HTTP error response.
pub fn classify_rpc_err(err: RpcErr) -> Response {
    EngineRestError::from(err).into()
}
