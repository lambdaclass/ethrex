//! SSZ body extractor for engine REST request handlers.

use axum::RequestExt;
use axum::body::Body;
use axum::extract::FromRequest;
use axum::http::{Request, header};
use http_body_util::LengthLimitError;
use libssz::SszDecode;

use crate::engine_rest::CONTENT_TYPE_OCTET_STREAM;
use crate::engine_rest::error::ProblemJson;

/// Axum extractor that reads an SSZ-encoded request body into `T`.
///
/// Errors map to RFC 7807 responses:
/// - missing or wrong Content-Type → 415 unsupported-media-type
/// - body exceeds the configured DefaultBodyLimit → 413 request-too-large
/// - SSZ decode failure → 400 ssz-decode-error
pub struct Ssz<T>(pub T);

impl<T, S> FromRequest<S> for Ssz<T>
where
    T: SszDecode + Send + 'static,
    S: Send + Sync,
{
    type Rejection = ProblemJson;

    async fn from_request(req: Request<Body>, _state: &S) -> Result<Self, Self::Rejection> {
        let content_type_ok = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| {
                let primary = s.split(';').next().unwrap_or("").trim();
                primary.eq_ignore_ascii_case(CONTENT_TYPE_OCTET_STREAM)
            })
            .unwrap_or(false);

        if !content_type_ok {
            return Err(ProblemJson::unsupported_media_type(
                "expected Content-Type: application/octet-stream",
            ));
        }

        // Use with_limited_body() to honour the DefaultBodyLimit middleware. Without
        // this call the raw body is read directly and the configured cap has no effect.
        let body = req.with_limited_body().into_body();
        let bytes = match axum::body::to_bytes(body, usize::MAX).await {
            Ok(b) => b,
            Err(err) => {
                if is_length_limit_error(&err) {
                    return Err(ProblemJson::request_too_large(
                        "request body exceeds configured limit",
                    ));
                }
                return Err(ProblemJson::bad_request(&format!(
                    "failed to read request body: {err}"
                )));
            }
        };

        T::from_ssz_bytes(&bytes)
            .map(Ssz)
            .map_err(|err| ProblemJson::ssz_decode_error(&format!("SSZ decode failed: {err}")))
    }
}

/// Decode an SSZ-encoded byte slice into `T`, mapping decode failures to
/// `ProblemJson::ssz_decode_error(...)`. Use when the target type isn't
/// statically known at the extractor level (i.e., it depends on a path/header
/// parameter).
pub fn decode_ssz<T: libssz::SszDecode>(bytes: &[u8]) -> Result<T, ProblemJson> {
    T::from_ssz_bytes(bytes)
        .map_err(|err| ProblemJson::ssz_decode_error(&format!("SSZ decode failed: {err}")))
}

/// Walk the error source chain to detect a `LengthLimitError`.
///
/// When `with_limited_body()` wraps the body and the limit is exceeded, axum
/// re-boxes the error once (via `Body::new` → `map_err(axum_core::Error::new)`),
/// so the `LengthLimitError` may be one or two source levels deep.
pub(crate) fn is_length_limit_error(err: &axum::Error) -> bool {
    let mut source: Option<&dyn std::error::Error> = std::error::Error::source(err);
    while let Some(s) = source {
        if s.downcast_ref::<LengthLimitError>().is_some() {
            return true;
        }
        source = s.source();
    }
    false
}
