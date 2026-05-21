//! POST /engine/v1/client/version.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use tracing::debug;

use crate::engine_rest::error::{EngineError, EngineRestError};
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::client_version::{
    ClientVersionV1 as SszClientVersionV1, GetClientVersionV1Request, GetClientVersionV1Response,
};
use crate::rpc::ClientVersion;

fn ssz_from_node_client_version(cv: &ClientVersion) -> Result<SszClientVersionV1, EngineRestError> {
    // The SSZ wire requires exactly 4 commit bytes; if the build metadata
    // isn't a valid 8-char hex prefix (e.g. a `vergen` "unknown" fallback or
    // a non-hex tag like "dirty"), fall back to zeros rather than 500.
    let commit_hex = cv.commit.get(..8).unwrap_or("00000000");
    let commit: [u8; 4] = hex::decode(commit_hex)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .unwrap_or([0u8; 4]);
    let code = b"EX".to_vec();
    let name = cv.name.as_bytes().to_vec();
    let version = format!("v{}", cv.version).into_bytes();
    Ok(SszClientVersionV1 {
        code: code
            .try_into()
            .map_err(|_| EngineRestError::internal("client code overflow"))?,
        name: name
            .try_into()
            .map_err(|_| EngineRestError::internal("client name overflow"))?,
        version: version
            .try_into()
            .map_err(|_| EngineRestError::internal("client version overflow"))?,
        commit,
    })
}

pub async fn client_version(
    State(cv): State<ClientVersion>,
    Ssz(_req): Ssz<GetClientVersionV1Request>,
) -> Response {
    debug!("engine REST: /engine/v1/client/version");
    let entry = match ssz_from_node_client_version(&cv) {
        Ok(v) => v,
        Err(e) => return e.into(),
    };
    let versions = match vec![entry].try_into() {
        Ok(v) => v,
        Err(_) => return EngineError::internal("versions overflow MAX_CLIENT_VERSIONS"),
    };
    SszBody(GetClientVersionV1Response { versions }).into_response()
}
