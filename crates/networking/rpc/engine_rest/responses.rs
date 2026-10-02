//! SSZ response body wrapper for engine REST handlers.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use libssz::SszEncode;

use crate::engine_rest::CONTENT_TYPE_OCTET_STREAM;

/// Wraps a value implementing `SszEncode` and serves it as an SSZ-encoded
/// `application/octet-stream` response body with status 200.
pub struct SszBody<T>(pub T);

impl<T: SszEncode> IntoResponse for SszBody<T> {
    fn into_response(self) -> Response {
        let bytes = self.0.to_ssz();
        let mut resp = (StatusCode::OK, bytes).into_response();
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(CONTENT_TYPE_OCTET_STREAM),
        );
        resp
    }
}
