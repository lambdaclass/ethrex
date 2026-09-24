//! GET /identity — returns the EL's ClientVersion array in JSON.
//!
//! Spec: replaces `engine_getClientVersionV1`. CL identifies itself via the
//! `X-Engine-Client-Version` request header, captured by auth middleware.

use axum::Json;
use axum::extract::State;

use crate::engine::client_version::ClientVersionV1;
use crate::rpc::ClientVersion;

pub async fn get_identity(State(cv): State<ClientVersion>) -> Json<Vec<ClientVersionV1>> {
    Json(vec![ClientVersionV1::from_client_version(&cv)])
}
