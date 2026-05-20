//! Shared helpers for engine REST handlers.

use axum::http::{HeaderMap, header};

use crate::engine_rest::error::ProblemJson;

const CONTENT_TYPE_OCTET_STREAM: &str = "application/octet-stream";

/// Validate the request `Content-Type` is `application/octet-stream` (case-
/// insensitive, allows trailing parameters). Returns `ProblemJson` (415) on
/// mismatch/missing/non-ASCII.
pub fn check_content_type(headers: &HeaderMap) -> Result<(), ProblemJson> {
    let ok = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            let primary = s.split(';').next().unwrap_or("").trim();
            primary.eq_ignore_ascii_case(CONTENT_TYPE_OCTET_STREAM)
        })
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(ProblemJson::unsupported_media_type(
            "expected Content-Type: application/octet-stream",
        ))
    }
}
