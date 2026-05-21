//! SSZ response wrapper.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use libssz::SszEncode;

use crate::engine_rest::extractors::SSZ_CONTENT_TYPE;

/// SSZ-encoded 200 OK response.
pub struct SszBody<T>(pub T);

impl<T: SszEncode> IntoResponse for SszBody<T> {
    fn into_response(self) -> Response {
        let mut bytes = Vec::with_capacity(self.0.encoded_len());
        self.0.ssz_append(&mut bytes);
        let mut resp = Response::new(Body::from(bytes));
        *resp.status_mut() = StatusCode::OK;
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(SSZ_CONTENT_TYPE),
        );
        resp
    }
}

/// Add `Cache-Control: no-store`; payloads keep changing until the slot deadline.
pub fn add_no_store(mut resp: Response) -> Response {
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}
