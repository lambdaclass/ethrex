//! RFC 7807 problem+json error responses for the engine REST API.
//!
//! Per the latest spec (execution-apis #793, `refactor.md § Error model`) the
//! body carries only two fields: a required `type` (a relative URI rooted at
//! `/engine-api/errors/...`, stable across releases, which CLs branch on) and an
//! optional human-readable `detail`. The HTTP status code travels in the status
//! line, not the body. `title`/`status`/`instance` from the generic RFC 7807
//! shape are dropped.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

const CONTENT_TYPE_PROBLEM_JSON: &str = "application/problem+json";

/// RFC 7807 problem details, narrowed to the two fields the engine REST spec
/// uses. `status` is retained as a (non-serialized) field so call sites and
/// tests can inspect the HTTP status without re-parsing the response.
#[derive(Debug, Serialize)]
pub struct ProblemJson {
    #[serde(rename = "type")]
    pub typ: String,
    /// HTTP status code. Carried in the response status line, not the JSON body.
    #[serde(skip)]
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ProblemJson {
    /// Build a problem with an explicit spec `type` URI and HTTP status.
    pub fn new(status: StatusCode, typ: &str, detail: Option<&str>) -> Self {
        Self {
            typ: typ.to_string(),
            status: status.as_u16(),
            detail: detail.map(str::to_string),
        }
    }

    /// 400 — request shape is wrong (missing required field, bad value, etc.).
    pub fn bad_request(detail: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "/engine-api/errors/invalid-request",
            Some(detail),
        )
    }

    /// 400 — SSZ decode failed.
    pub fn ssz_decode_error(detail: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "/engine-api/errors/ssz-decode-error",
            Some(detail),
        )
    }

    /// 400 — `Eth-Execution-Version` missing, unknown, or unsupported.
    pub fn unsupported_fork(detail: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "/engine-api/errors/unsupported-fork",
            Some(detail),
        )
    }

    /// 401 — JWT bearer auth failed. (Not in the spec error table; auth is
    /// transport-level and shared with the legacy JSON-RPC engine API.)
    pub fn unauthorized(detail: &str) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "/engine-api/errors/unauthorized",
            Some(detail),
        )
    }

    /// 404 — `payloadId` does not exist.
    pub fn unknown_payload(detail: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "/engine-api/errors/unknown-payload",
            Some(detail),
        )
    }

    /// 409 — forkchoice state is inconsistent (e.g. finalized not an ancestor
    /// of head). Maps to the legacy `-38002`.
    pub fn invalid_forkchoice(detail: &str) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "/engine-api/errors/invalid-forkchoice",
            Some(detail),
        )
    }

    /// 409 — reorg depth exceeds the EL's limit. Maps to the legacy `-38006`.
    pub fn reorg_too_deep(detail: &str) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "/engine-api/errors/reorg-too-deep",
            Some(detail),
        )
    }

    /// 413 — body exceeds an advertised `limits.*` value.
    pub fn request_too_large(detail: &str) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "/engine-api/errors/request-too-large",
            Some(detail),
        )
    }

    /// 415 — request `Content-Type` does not match the endpoint's encoding.
    pub fn unsupported_media_type(detail: &str) -> Self {
        Self::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "/engine-api/errors/unsupported-media-type",
            Some(detail),
        )
    }

    /// 422 — body decoded fine but has invalid values.
    pub fn invalid_body(detail: &str) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "/engine-api/errors/invalid-body",
            Some(detail),
        )
    }

    /// 422 — `payload_attributes` validation failed. Maps to the legacy `-38003`.
    pub fn invalid_attributes(detail: &str) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "/engine-api/errors/invalid-attributes",
            Some(detail),
        )
    }

    /// 501 — endpoint registered but not yet implemented.
    pub fn not_implemented(detail: &str) -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            "/engine-api/errors/not-implemented",
            Some(detail),
        )
    }

    /// 500 — unrecoverable server error; `detail` carries the message.
    pub fn internal(detail: &str) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "/engine-api/errors/internal",
            Some(detail),
        )
    }
}

impl IntoResponse for ProblemJson {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_vec(&self).expect("ProblemJson serializes");
        let mut resp = (status, body).into_response();
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(CONTENT_TYPE_PROBLEM_JSON),
        );
        resp
    }
}
