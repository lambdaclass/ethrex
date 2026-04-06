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

use crate::authentication::authenticate;
use crate::rpc::RpcApiContext;
use crate::types::payload::{ExecutionPayload, PayloadValidationStatus};
use crate::utils::RpcErr;

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
        RpcErr::UnsupportedFork(msg) => {
            json_error_response(-38005, msg)
        }
        RpcErr::Internal(msg) => json_error_response(-32603, msg),
        _ => json_error_response(-32603, &format!("{err:?}")),
    }
}


/// `POST /new-payload-with-witness`
///
/// Accepts JSON body: `[executionPayload, expectedBlobVersionedHashes, parentBeaconBlockRoot, executionRequests]`
/// Returns SSZ-encoded `NewPayloadWithWitnessResponseV1`.
pub async fn handle_new_payload_with_witness(
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
        return json_error_response(
            -32602,
            &format!("Expected 4 params, got {}", params.len()),
        );
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

    let execution_requests: Vec<EncodedRequests> = match serde_json::from_value(params[3].clone())
    {
        Ok(r) => r,
        Err(_) => {
            return json_error_response(-32602, "Invalid executionRequests");
        }
    };

    if let Err(e) = payload::validate_execution_payload_v4_public(&exec_payload) {
        return rpc_err_to_response(e);
    }

    if let Err(e) = payload::validate_execution_requests_public(&execution_requests) {
        return rpc_err_to_response(e);
    }

    let requests_hash = compute_requests_hash(&execution_requests);

    let raw_bal_hash = params[0]
        .get("blockAccessList")
        .and_then(|v| v.as_str())
        .and_then(|hex_str| hex::decode(hex_str.trim_start_matches("0x")).ok())
        .map(|bytes| ethrex_common::utils::keccak(bytes));

    let block = match exec_payload.clone().into_block(
        Some(parent_beacon_block_root),
        Some(requests_hash),
        raw_bal_hash,
    ) {
        Ok(b) => b,
        Err(e) => {
            // Invalid block construction → return SSZ response with INVALID status.
            let resp = SszNewPayloadWithWitnessResponse::from_status(
                PayloadValidationStatus::Invalid,
                None,
                Some(e.to_string()),
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
    if let Some(ref b) = bal {
        if let Err(err) = b.validate_ordering() {
            let resp = SszNewPayloadWithWitnessResponse::from_status(
                PayloadValidationStatus::Invalid,
                None,
                Some(err),
                None,
            );
            return ssz_response(resp);
        }
    }

    let block_hash = exec_payload.block_hash;
    let actual_block_hash = block.hash();
    if block_hash != actual_block_hash {
        let resp = SszNewPayloadWithWitnessResponse::from_status(
            PayloadValidationStatus::Invalid,
            None,
            Some(format!(
                "Invalid block hash. Expected {actual_block_hash:#x}, got {block_hash:#x}"
            )),
            None,
        );
        return ssz_response(resp);
    }

    let blob_versioned_hashes: Vec<H256> = block
        .body
        .transactions
        .iter()
        .flat_map(|tx| tx.blob_versioned_hashes())
        .collect();

    if expected_blob_versioned_hashes != blob_versioned_hashes {
        let resp = SszNewPayloadWithWitnessResponse::from_status(
            PayloadValidationStatus::Invalid,
            None,
            Some("Invalid blob_versioned_hashes".to_string()),
            None,
        );
        return ssz_response(resp);
    }

    let payload_result = payload::add_block_with_witness(&context, block, bal).await;

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
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("failed to build SSZ response")
        .into_response()
}
