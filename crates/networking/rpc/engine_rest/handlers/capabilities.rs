//! POST /engine/v1/capabilities.

use axum::response::{IntoResponse, Response};
use libssz_types::SszList;

use crate::engine::CAPABILITIES;
use crate::engine_rest::SSZ_REST_CAPABILITIES;
use crate::engine_rest::error::EngineError;
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::capabilities::{
    ExchangeCapabilitiesRequest, ExchangeCapabilitiesResponse,
};
use crate::engine_rest::types::common::MAX_CAPABILITY_NAME_LENGTH;

pub async fn capabilities(Ssz(_req): Ssz<ExchangeCapabilitiesRequest>) -> Response {
    let mut all: Vec<&'static str> = CAPABILITIES.to_vec();
    all.extend_from_slice(SSZ_REST_CAPABILITIES);

    let mut inner: Vec<SszList<u8, MAX_CAPABILITY_NAME_LENGTH>> = Vec::with_capacity(all.len());
    for cap in &all {
        let bytes: Vec<u8> = cap.as_bytes().to_vec();
        let entry = match bytes.try_into() {
            Ok(e) => e,
            Err(_) => {
                return EngineError::internal(&format!(
                    "capability name '{cap}' exceeds MAX_CAPABILITY_NAME_LENGTH"
                ));
            }
        };
        inner.push(entry);
    }
    let capabilities = match inner.try_into() {
        Ok(c) => c,
        Err(_) => return EngineError::internal("capabilities list overflow"),
    };
    SszBody(ExchangeCapabilitiesResponse { capabilities }).into_response()
}
