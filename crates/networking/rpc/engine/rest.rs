//! REST bindings for the Engine API.
//! The endpoint accepts the same JSON parameters as `engine_newPayloadV5`,
//! executes the block, generates a witness, and returns the result as
//! SSZ-encoded bytes (`Content-Type: application/octet-stream`).

use axum::{
    Json,
    body::Body,
    extract::State,
    http::{Response, StatusCode, header},
    response::IntoResponse,
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use libssz::SszEncode;
use libssz_derive::SszEncode;
use serde_json::Value;

use ethrex_common::H256;
use ethrex_common::types::block_execution_witness::RpcExecutionWitness;
use ethrex_common::types::requests::{EncodedRequests, compute_requests_hash};

use crate::{authentication::authenticate, engine::payload::get_block_from_payload};
use crate::{engine::payload::handle_new_payload_v3_with_witness, rpc::RpcApiContext};
use crate::{engine::payload::validate_execution_payload_v3, utils::RpcErr};
use crate::{
    engine::payload::{handle_new_payload_v4_with_witness, validate_execution_payload_v4},
    types::payload::{ExecutionPayload, PayloadValidationStatus},
};

use super::payload;

/// `Union[None, ByteVector[32]]` for `latest_valid_hash`.
#[derive(SszEncode)]
#[ssz(enum_behaviour = "union")]
pub enum OptionalHash {
    None,
    Hash([u8; 32]),
}

/// `Union[None, List[uint8, VALIDATION_ERROR_MAX]]` for `validation_error`.
#[derive(SszEncode)]
#[ssz(enum_behaviour = "union")]
pub enum OptionalValidationError {
    None,
    Error(Vec<u8>),
}

/// Maximum number of `uint8` elements for validation_error.
const VALIDATION_ERROR_MAX: usize = 8192;

/// SSZ `Container` for the top-level response.
///
/// ```text
/// status:            uint8
/// latest_valid_hash: Union[None, ByteVector[32]]
/// validation_error:  Union[None, List[uint8, VALIDATION_ERROR_MAX]]
/// witness:           List[uint8, MAX_WITNESS_BYTES]
/// ```
#[derive(SszEncode)]
pub struct SszNewPayloadWithWitnessResponse {
    pub status: u8,
    pub latest_valid_hash: OptionalHash,
    pub validation_error: OptionalValidationError,
    pub witness: Vec<u8>,
}

/// SSZ `Container` for `ExecutionWitnessV1`.
///
/// ```text
/// state:   List[List[uint8, MAX_WITNESS_ITEM_BYTES], MAX_WITNESS_ITEMS]
/// keys:    List[List[uint8, MAX_WITNESS_ITEM_BYTES], MAX_WITNESS_ITEMS]
/// codes:   List[List[uint8, MAX_WITNESS_ITEM_BYTES], MAX_WITNESS_ITEMS]
/// headers: List[List[uint8, MAX_WITNESS_ITEM_BYTES], MAX_WITNESS_ITEMS]
/// ```
#[derive(SszEncode)]
pub struct SszExecutionWitnessV1 {
    pub state: Vec<Vec<u8>>,
    pub keys: Vec<Vec<u8>>,
    pub codes: Vec<Vec<u8>>,
    pub headers: Vec<Vec<u8>>,
}

impl SszNewPayloadWithWitnessResponse {
    /// Build from a validation status + optional witness.
    fn from_status(
        status: PayloadValidationStatus,
        latest_valid_hash: Option<H256>,
        validation_error: Option<String>,
        witness: Option<RpcExecutionWitness>,
    ) -> Self {
        let status_byte = match status {
            PayloadValidationStatus::Valid => 0u8,
            PayloadValidationStatus::Invalid => 1u8,
            PayloadValidationStatus::Syncing => 2u8,
            PayloadValidationStatus::Accepted => 3u8,
        };

        let ssz_hash = match latest_valid_hash {
            Some(h) => OptionalHash::Hash(h.0),
            None => OptionalHash::None,
        };

        let ssz_error = match validation_error {
            Some(msg) => {
                let mut bytes = msg.into_bytes();
                if bytes.len() > VALIDATION_ERROR_MAX {
                    let mut end = VALIDATION_ERROR_MAX;
                    while end > 0 && (bytes[end] & 0xC0) == 0x80 {
                        end -= 1;
                    }
                    bytes.truncate(end);
                }
                OptionalValidationError::Error(bytes)
            }
            None => OptionalValidationError::None,
        };

        let witness_bytes = if status_byte == 0 {
            match witness {
                Some(w) => {
                    let ssz_witness = SszExecutionWitnessV1::from(w);
                    ssz_witness.to_ssz()
                }
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };

        Self {
            status: status_byte,
            latest_valid_hash: ssz_hash,
            validation_error: ssz_error,
            witness: witness_bytes,
        }
    }
}

impl From<RpcExecutionWitness> for SszExecutionWitnessV1 {
    fn from(w: RpcExecutionWitness) -> Self {
        Self {
            state: w.state.into_iter().map(|b| b.to_vec()).collect(),
            keys: w.keys.into_iter().map(|b| b.to_vec()).collect(),
            codes: w.codes.into_iter().map(|b| b.to_vec()).collect(),
            headers: w.headers.into_iter().map(|b| b.to_vec()).collect(),
        }
    }
}

/// JSON error body matching the JSON-RPC `error` shape.
fn json_error_response(code: i32, message: &str) -> axum::response::Response {
    let status = match code {
        -32700 | -32602 => StatusCode::BAD_REQUEST,
        -38005 => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let body = serde_json::json!({
        "code": code,
        "message": message,
    });
    (status, Json(body)).into_response()
}

fn rpc_err_to_response(err: RpcErr) -> axum::response::Response {
    match &err {
        RpcErr::BadParams(msg) | RpcErr::WrongParam(msg) | RpcErr::InvalidPayload(msg) => {
            json_error_response(-32602, msg)
        }
        RpcErr::UnsupportedFork(msg) => json_error_response(-38005, msg),
        RpcErr::Internal(msg) => json_error_response(-32603, msg),
        _ => json_error_response(-32603, &format!("{err:?}")),
    }
}

/// `POST /new-payload-with-witness-v4`
pub async fn handle_new_payload_with_witness_v4(
    State(context): State<RpcApiContext>,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    if let Err(_err) = authenticate(&context.node_data.jwt_secret, auth_header) {
        return json_error_response(-32700, "JWT authentication failed");
    }

    let params: Vec<Value> = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return json_error_response(-32700, &format!("Parse error: {e}"));
        }
    };

    if params.len() != 4 {
        return json_error_response(-32602, &format!("Expected 4 params, got {}", params.len()));
    }

    let exec_payload: ExecutionPayload = match serde_json::from_value(params[0].clone()) {
        Ok(p) => p,
        Err(_) => {
            return json_error_response(-32602, "Invalid executionPayload");
        }
    };

    let expected_blob_versioned_hashes: Vec<H256> = match serde_json::from_value(params[1].clone())
    {
        Ok(h) => h,
        Err(_) => {
            return json_error_response(-32602, "Invalid expectedBlobVersionedHashes");
        }
    };

    let parent_beacon_block_root: H256 = match serde_json::from_value(params[2].clone()) {
        Ok(r) => r,
        Err(_) => {
            return json_error_response(-32602, "Invalid parentBeaconBlockRoot");
        }
    };

    let execution_requests: Vec<EncodedRequests> = match serde_json::from_value(params[3].clone()) {
        Ok(r) => r,
        Err(_) => {
            return json_error_response(-32602, "Invalid executionRequests");
        }
    };

    if let Err(e) = payload::validate_execution_requests(&execution_requests) {
        return rpc_err_to_response(e);
    }

    let requests_hash = compute_requests_hash(&execution_requests);

    let block = match get_block_from_payload(
        &exec_payload,
        Some(parent_beacon_block_root),
        Some(requests_hash),
        None,
    ) {
        Ok(block) => block,
        Err(err) => {
            let resp = SszNewPayloadWithWitnessResponse::from_status(
                PayloadValidationStatus::Invalid,
                None,
                Some(err.to_string()),
                None,
            );
            return ssz_response(resp);
        }
    };

    let chain_config = context.storage.get_chain_config();

    if !chain_config.is_prague_activated(block.header.timestamp) {
        return rpc_err_to_response(RpcErr::UnsupportedFork(format!(
            "{:?}",
            chain_config.get_fork(block.header.timestamp)
        )));
    }
    // We use v3 since the execution payload remains the same.
    if let Err(e) = validate_execution_payload_v3(&exec_payload) {
        return rpc_err_to_response(e);
    }
    let payload_result = handle_new_payload_v3_with_witness(
        &exec_payload,
        context,
        block,
        expected_blob_versioned_hashes.clone(),
        None,
    )
    .await;

    match payload_result {
        Ok(payload_status) => {
            let resp = SszNewPayloadWithWitnessResponse::from_status(
                payload_status.status,
                payload_status.latest_valid_hash,
                payload_status.validation_error,
                payload_status.witness,
            );
            ssz_response(resp)
        }
        Err(rpc_err) => rpc_err_to_response(rpc_err),
    }
}

/// `POST /new-payload-with-witness-v5`
pub async fn handle_new_payload_with_witness_v5(
    State(context): State<RpcApiContext>,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    if let Err(_err) = authenticate(&context.node_data.jwt_secret, auth_header) {
        return json_error_response(-32700, "JWT authentication failed");
    }

    let params: Vec<Value> = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return json_error_response(-32700, &format!("Parse error: {e}"));
        }
    };

    if params.len() != 4 {
        return json_error_response(-32602, &format!("Expected 4 params, got {}", params.len()));
    }

    // Extract the raw BAL hash from the JSON payload before deserialization.
    // We hash the raw RLP bytes as-received to preserve the exact encoding
    // (including any ordering) for accurate block hash validation.
    let Ok(raw_bal_hash) = params[0]
        .get("blockAccessList")
        .map(|v| {
            let hex_str = v
                .as_str()
                .ok_or(RpcErr::WrongParam("blockAccessList".to_string()))?;
            let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                .map_err(|_| RpcErr::WrongParam("blockAccessList".to_string()))?;
            Ok::<_, RpcErr>(ethrex_common::utils::keccak(bytes))
        })
        .transpose()
    else {
        return json_error_response(-32602, "Invalid blockAccessList");
    };

    let exec_payload: ExecutionPayload = match serde_json::from_value(params[0].clone()) {
        Ok(p) => p,
        Err(_) => {
            return json_error_response(-32602, "Invalid executionPayload");
        }
    };

    let expected_blob_versioned_hashes: Vec<H256> = match serde_json::from_value(params[1].clone())
    {
        Ok(h) => h,
        Err(_) => {
            return json_error_response(-32602, "Invalid expectedBlobVersionedHashes");
        }
    };

    let parent_beacon_block_root: H256 = match serde_json::from_value(params[2].clone()) {
        Ok(r) => r,
        Err(_) => {
            return json_error_response(-32602, "Invalid parentBeaconBlockRoot");
        }
    };

    let execution_requests: Vec<EncodedRequests> = match serde_json::from_value(params[3].clone()) {
        Ok(r) => r,
        Err(_) => {
            return json_error_response(-32602, "Invalid executionRequests");
        }
    };

    if let Err(e) = validate_execution_payload_v4(&exec_payload) {
        return rpc_err_to_response(e);
    }

    if let Err(e) = payload::validate_execution_requests(&execution_requests) {
        return rpc_err_to_response(e);
    }

    let requests_hash = compute_requests_hash(&execution_requests);
    // Use the hash computed from the raw RLP bytes as-received.
    // This preserves the exact encoding (including any ordering) from the payload,
    // so the block hash check correctly detects BAL corruption.
    let block_access_list_hash = raw_bal_hash;

    let block = match get_block_from_payload(
        &exec_payload,
        Some(parent_beacon_block_root),
        Some(requests_hash),
        block_access_list_hash,
    ) {
        Ok(block) => block,
        Err(err) => {
            let resp = SszNewPayloadWithWitnessResponse::from_status(
                PayloadValidationStatus::Invalid,
                None,
                Some(err.to_string()),
                None,
            );
            return ssz_response(resp);
        }
    };

    let chain_config = context.storage.get_chain_config();

    if !chain_config.is_amsterdam_activated(block.header.timestamp) {
        return rpc_err_to_response(RpcErr::UnsupportedFork(format!(
            "{:?}",
            chain_config.get_fork(block.header.timestamp)
        )));
    }

    let bal = exec_payload.block_access_list.clone();
    let payload_result = handle_new_payload_v4_with_witness(
        &exec_payload,
        context,
        block,
        expected_blob_versioned_hashes.clone(),
        bal,
    )
    .await;

    match payload_result {
        Ok(payload_status) => {
            let resp = SszNewPayloadWithWitnessResponse::from_status(
                payload_status.status,
                payload_status.latest_valid_hash,
                payload_status.validation_error,
                payload_status.witness,
            );
            ssz_response(resp)
        }
        Err(rpc_err) => rpc_err_to_response(rpc_err),
    }
}

/// Produce a `200 OK` response with `Content-Type: application/octet-stream`.
fn ssz_response(resp: SszNewPayloadWithWitnessResponse) -> axum::response::Response {
    let bytes = resp.to_ssz();
    println!("SSZ response length: {:?}", bytes.len());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("failed to build SSZ response")
        .into_response()
}
