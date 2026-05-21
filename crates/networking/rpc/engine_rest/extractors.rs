//! SSZ request extractor + Content-Type guard.

use axum::body::Bytes;
use axum::extract::FromRequest;
use axum::http::{self, HeaderMap, Request};
use libssz::SszDecode;

use crate::engine_rest::error::EngineRestError;

pub const SSZ_CONTENT_TYPE: &str = "application/octet-stream";

/// Validate that the incoming request advertises SSZ bytes.
pub fn check_ssz_content_type(headers: &HeaderMap) -> Result<(), EngineRestError> {
    match headers.get(http::header::CONTENT_TYPE) {
        Some(ct) if ct.as_bytes().starts_with(SSZ_CONTENT_TYPE.as_bytes()) => Ok(()),
        Some(ct) => Err(EngineRestError::bad_request(format!(
            "unsupported Content-Type {:?}, expected {SSZ_CONTENT_TYPE}",
            ct.to_str().unwrap_or("<binary>"),
        ))),
        None => Err(EngineRestError::bad_request(format!(
            "missing Content-Type, expected {SSZ_CONTENT_TYPE}"
        ))),
    }
}

/// SSZ-decode the request body. Returns an `EngineRestError::bad_request`
/// (mapped to a 400 response at the handler boundary) on failure.
pub fn decode_ssz<T: SszDecode>(bytes: &[u8]) -> Result<T, EngineRestError> {
    T::from_ssz_bytes(bytes)
        .map_err(|e| EngineRestError::bad_request(format!("invalid SSZ: {e:?}")))
}

/// Axum extractor that enforces Content-Type and SSZ-decodes the body.
pub struct Ssz<T>(pub T);

impl<T, S> FromRequest<S> for Ssz<T>
where
    T: SszDecode + Send + 'static,
    S: Send + Sync,
{
    type Rejection = EngineRestError;

    async fn from_request(
        req: Request<axum::body::Body>,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        check_ssz_content_type(req.headers())?;
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|e| EngineRestError::bad_request(format!("failed to read body: {e}")))?;
        let value = decode_ssz::<T>(&bytes)?;
        Ok(Ssz(value))
    }
}
