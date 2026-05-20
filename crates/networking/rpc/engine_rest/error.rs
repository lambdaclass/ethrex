//! RFC 7807 problem+json error responses for the engine REST API.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

const CONTENT_TYPE_PROBLEM_JSON: &str = "application/problem+json";

/// RFC 7807 problem details. Used for every non-success engine REST response.
#[derive(Debug, Serialize)]
pub struct ProblemJson {
    #[serde(rename = "type")]
    pub typ: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

impl ProblemJson {
    pub fn new(status: StatusCode, title: &str, detail: Option<&str>) -> Self {
        Self {
            typ: "about:blank".to_string(),
            title: title.to_string(),
            status: status.as_u16(),
            detail: detail.map(str::to_string),
            instance: None,
        }
    }

    pub fn bad_request(detail: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "Bad Request", Some(detail))
    }

    pub fn unauthorized(detail: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Unauthorized", Some(detail))
    }

    pub fn not_found(detail: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "Not Found", Some(detail))
    }

    pub fn unsupported_media_type(detail: &str) -> Self {
        Self::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Unsupported Media Type",
            Some(detail),
        )
    }

    pub fn payload_too_large(detail: &str) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Payload Too Large",
            Some(detail),
        )
    }

    pub fn unprocessable_entity(detail: &str) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Unprocessable Entity",
            Some(detail),
        )
    }

    pub fn conflict(detail: &str) -> Self {
        Self::new(StatusCode::CONFLICT, "Conflict", Some(detail))
    }

    pub fn not_implemented(detail: &str) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, "Not Implemented", Some(detail))
    }

    pub fn internal(detail: &str) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
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
