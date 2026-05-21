//! POST /engine/v{1..5}/payloads and GET /engine/v{1..6}/payloads/{payload_id}.

use std::str::FromStr;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use ethrex_common::H256;
use ethrex_common::types::Fork;
use ethrex_common::types::requests::compute_requests_hash;
use ethrex_common::utils::keccak;

use crate::engine::payload::{
    get_payload, handle_new_payload_v1_v2, handle_new_payload_v3, handle_new_payload_v4,
    validate_execution_requests, validate_fork, validate_payload_v1_v2,
};
use crate::engine_rest::conversions::{
    blobs_bundle_to_ssz_v1, blobs_bundle_to_ssz_v2, encoded_requests_to_ssz,
    json_payload_status_to_ssz, json_to_execution_payload_v1, json_to_execution_payload_v2,
    json_to_execution_payload_v3, json_to_execution_payload_v4, ssz_blob_hashes_to_vec,
    ssz_payload_v1_to_block, ssz_payload_v2_to_block, ssz_payload_v3_to_block,
    ssz_payload_v4_to_block, ssz_to_encoded_requests,
};
use crate::engine_rest::error::{EngineError, EngineRestError, classify_rpc_err};
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::responses::{SszBody, add_no_store};
use crate::engine_rest::types::common::{PayloadId, u256_to_uint256_le};
use crate::engine_rest::types::get_payload::{
    GetPayloadResponseV2, GetPayloadResponseV3, GetPayloadResponseV4, GetPayloadResponseV5,
    GetPayloadResponseV6,
};
use crate::engine_rest::types::new_payload::{
    NewPayloadV1Request, NewPayloadV2Request, NewPayloadV3Request, NewPayloadV4Request,
    NewPayloadV5Request,
};
use crate::rpc::RpcApiContext;
use crate::types::payload::{ExecutionPayload as JsonExecutionPayload, PayloadStatus};

pub async fn new_payload_v1(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<NewPayloadV1Request>,
) -> Response {
    let expected_hash = H256::from(req.execution_payload.block_hash);
    let block = match ssz_payload_v1_to_block(req.execution_payload, None, None, None) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    if let Err(e) = validate_payload_v1_v2(&block, &ctx) {
        return classify_rpc_err(e);
    }
    let status = match handle_new_payload_v1_v2(expected_hash, block, ctx, None).await {
        Ok(s) => s,
        Err(e) => return classify_rpc_err(e),
    };
    ssz_status_response(&status)
}

pub async fn new_payload_v2(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<NewPayloadV2Request>,
) -> Response {
    let payload = req.execution_payload;
    let expected_hash = H256::from(payload.block_hash);
    // SSZ always carries a (possibly empty) withdrawals list; pre-Shanghai
    // JSON-RPC's V2 rejects any withdrawals, so do the same here.
    let timestamp = payload.timestamp;
    let has_withdrawals = !payload.withdrawals.is_empty();
    if pre_shanghai(&ctx, timestamp) && has_withdrawals {
        return EngineError::unprocessable("pre-Shanghai payload must not carry withdrawals");
    }
    let block = match ssz_payload_v2_to_block(payload, None, None, None) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    if let Err(e) = validate_payload_v1_v2(&block, &ctx) {
        return classify_rpc_err(e);
    }
    let status = match handle_new_payload_v1_v2(expected_hash, block, ctx, None).await {
        Ok(s) => s,
        Err(e) => return classify_rpc_err(e),
    };
    ssz_status_response(&status)
}

fn pre_shanghai(ctx: &RpcApiContext, ts: u64) -> bool {
    !ctx.storage.get_chain_config().is_shanghai_activated(ts)
}

pub async fn new_payload_v3(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<NewPayloadV3Request>,
) -> Response {
    let expected_hash = H256::from(req.execution_payload.block_hash);
    let block = match ssz_payload_v3_to_block(
        req.execution_payload,
        Some(H256::from(req.parent_beacon_block_root)),
        None,
        None,
    ) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    if let Err(e) = validate_fork(&block, Fork::Cancun, &ctx) {
        return classify_rpc_err(e);
    }
    let expected = ssz_blob_hashes_to_vec(&req.expected_blob_versioned_hashes);
    let status = match handle_new_payload_v3(expected_hash, ctx, block, expected, None).await {
        Ok(s) => s,
        Err(e) => return classify_rpc_err(e),
    };
    ssz_status_response(&status)
}

pub async fn new_payload_v4(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<NewPayloadV4Request>,
) -> Response {
    let expected_hash = H256::from(req.execution_payload.block_hash);
    let exec_requests = ssz_to_encoded_requests(&req.execution_requests);
    if let Err(e) = validate_execution_requests(&exec_requests) {
        return classify_rpc_err(e);
    }
    let requests_hash = compute_requests_hash(&exec_requests);
    let block = match ssz_payload_v3_to_block(
        req.execution_payload,
        Some(H256::from(req.parent_beacon_block_root)),
        Some(requests_hash),
        None,
    ) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let chain_config = ctx.storage.get_chain_config();
    if !chain_config.is_prague_activated(block.header.timestamp) {
        return EngineError::unprocessable(&format!(
            "{:?}",
            chain_config.get_fork(block.header.timestamp)
        ));
    }
    let expected = ssz_blob_hashes_to_vec(&req.expected_blob_versioned_hashes);
    let status = match handle_new_payload_v3(expected_hash, ctx, block, expected, None).await {
        Ok(s) => s,
        Err(e) => return classify_rpc_err(e),
    };
    ssz_status_response(&status)
}

pub async fn new_payload_v5(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<NewPayloadV5Request>,
) -> Response {
    let expected_hash = H256::from(req.execution_payload.block_hash);
    // Hash the raw BAL bytes as-received: re-encoding through RLP can change
    // ordering and break the block-hash check.
    let raw_bal_hash = if req.execution_payload.block_access_list.is_empty() {
        None
    } else {
        Some(keccak(&req.execution_payload.block_access_list[..]))
    };

    let exec_requests = ssz_to_encoded_requests(&req.execution_requests);
    if let Err(e) = validate_execution_requests(&exec_requests) {
        return classify_rpc_err(e);
    }
    let requests_hash = compute_requests_hash(&exec_requests);

    let (block, bal) = match ssz_payload_v4_to_block(
        req.execution_payload,
        Some(H256::from(req.parent_beacon_block_root)),
        Some(requests_hash),
        raw_bal_hash,
    ) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let chain_config = ctx.storage.get_chain_config();
    if !chain_config.is_amsterdam_activated(block.header.timestamp) {
        return EngineError::unprocessable(&format!(
            "{:?}",
            chain_config.get_fork(block.header.timestamp)
        ));
    }
    let expected = ssz_blob_hashes_to_vec(&req.expected_blob_versioned_hashes);
    let status = match handle_new_payload_v4(expected_hash, ctx, block, expected, bal).await {
        Ok(s) => s,
        Err(e) => return classify_rpc_err(e),
    };
    ssz_status_response(&status)
}

fn parse_payload_id(s: &str) -> Result<PayloadId, EngineRestError> {
    PayloadId::from_str(s)
        .map_err(|e| EngineRestError::bad_request(format!("invalid payload_id: {e}")))
}

async fn fetch_payload(
    ctx: &RpcApiContext,
    id: PayloadId,
) -> Result<ethrex_common::types::payload::PayloadBundle, EngineRestError> {
    get_payload(id.as_u64(), ctx).await.map_err(Into::into)
}

pub async fn get_payload_v1(
    State(ctx): State<RpcApiContext>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_payload_id(&id_str) {
        Ok(id) => id,
        Err(e) => return e.into(),
    };
    let bundle = match fetch_payload(&ctx, id).await {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    if let Err(e) = validate_payload_v1_v2(&bundle.block, &ctx) {
        return classify_rpc_err(e);
    }
    let json = JsonExecutionPayload::from_block(bundle.block, None);
    match json_to_execution_payload_v1(&json) {
        Ok(p) => add_no_store(SszBody(p).into_response()),
        Err(e) => e.into(),
    }
}

pub async fn get_payload_v2(
    State(ctx): State<RpcApiContext>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_payload_id(&id_str) {
        Ok(id) => id,
        Err(e) => return e.into(),
    };
    let bundle = match fetch_payload(&ctx, id).await {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    if let Err(e) = validate_payload_v1_v2(&bundle.block, &ctx) {
        return classify_rpc_err(e);
    }
    let block_value = u256_to_uint256_le(bundle.block_value);
    let json = JsonExecutionPayload::from_block(bundle.block, None);
    let payload = match json_to_execution_payload_v2(&json) {
        Ok(p) => p,
        Err(e) => return e.into(),
    };
    add_no_store(
        SszBody(GetPayloadResponseV2 {
            execution_payload: payload,
            block_value,
        })
        .into_response(),
    )
}

pub async fn get_payload_v3(
    State(ctx): State<RpcApiContext>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_payload_id(&id_str) {
        Ok(id) => id,
        Err(e) => return e.into(),
    };
    let bundle = match fetch_payload(&ctx, id).await {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    if let Err(e) = validate_fork(&bundle.block, Fork::Cancun, &ctx) {
        return classify_rpc_err(e);
    }
    let block_value = u256_to_uint256_le(bundle.block_value);
    let blobs_bundle = match blobs_bundle_to_ssz_v1(bundle.blobs_bundle) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let json = JsonExecutionPayload::from_block(bundle.block, None);
    let payload = match json_to_execution_payload_v3(&json) {
        Ok(p) => p,
        Err(e) => return e.into(),
    };
    add_no_store(
        SszBody(GetPayloadResponseV3 {
            execution_payload: payload,
            block_value,
            blobs_bundle,
            should_override_builder: false,
        })
        .into_response(),
    )
}

pub async fn get_payload_v4(
    State(ctx): State<RpcApiContext>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_payload_id(&id_str) {
        Ok(id) => id,
        Err(e) => return e.into(),
    };
    let bundle = match fetch_payload(&ctx, id).await {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let chain_config = ctx.storage.get_chain_config();
    if !chain_config.is_prague_activated(bundle.block.header.timestamp) {
        return EngineError::unprocessable(&format!(
            "{:?}",
            chain_config.get_fork(bundle.block.header.timestamp)
        ));
    }
    if chain_config.is_osaka_activated(bundle.block.header.timestamp) {
        return EngineError::unprocessable(&format!("{:?}", Fork::Osaka));
    }
    let block_value = u256_to_uint256_le(bundle.block_value);
    let blobs_bundle = match blobs_bundle_to_ssz_v1(bundle.blobs_bundle) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let execution_requests = match encoded_requests_to_ssz(&bundle.requests) {
        Ok(r) => r,
        Err(e) => return e.into(),
    };
    let json = JsonExecutionPayload::from_block(bundle.block, None);
    let payload = match json_to_execution_payload_v3(&json) {
        Ok(p) => p,
        Err(e) => return e.into(),
    };
    add_no_store(
        SszBody(GetPayloadResponseV4 {
            execution_payload: payload,
            block_value,
            blobs_bundle,
            should_override_builder: false,
            execution_requests,
        })
        .into_response(),
    )
}

pub async fn get_payload_v5(
    State(ctx): State<RpcApiContext>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_payload_id(&id_str) {
        Ok(id) => id,
        Err(e) => return e.into(),
    };
    let bundle = match fetch_payload(&ctx, id).await {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let chain_config = ctx.storage.get_chain_config();
    if !chain_config.is_osaka_activated(bundle.block.header.timestamp) {
        return EngineError::unprocessable(&format!(
            "{:?}",
            chain_config.get_fork(bundle.block.header.timestamp)
        ));
    }
    let block_value = u256_to_uint256_le(bundle.block_value);
    let blobs_bundle = match blobs_bundle_to_ssz_v2(bundle.blobs_bundle) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let execution_requests = match encoded_requests_to_ssz(&bundle.requests) {
        Ok(r) => r,
        Err(e) => return e.into(),
    };
    let json = JsonExecutionPayload::from_block(bundle.block, bundle.block_access_list);
    let payload = match json_to_execution_payload_v3(&json) {
        Ok(p) => p,
        Err(e) => return e.into(),
    };
    add_no_store(
        SszBody(GetPayloadResponseV5 {
            execution_payload: payload,
            block_value,
            blobs_bundle,
            should_override_builder: false,
            execution_requests,
        })
        .into_response(),
    )
}

pub async fn get_payload_v6(
    State(ctx): State<RpcApiContext>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_payload_id(&id_str) {
        Ok(id) => id,
        Err(e) => return e.into(),
    };
    let bundle = match fetch_payload(&ctx, id).await {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let chain_config = ctx.storage.get_chain_config();
    if !chain_config.is_amsterdam_activated(bundle.block.header.timestamp) {
        return EngineError::unprocessable(&format!(
            "{:?}",
            chain_config.get_fork(bundle.block.header.timestamp)
        ));
    }
    let block_value = u256_to_uint256_le(bundle.block_value);
    let blobs_bundle = match blobs_bundle_to_ssz_v2(bundle.blobs_bundle) {
        Ok(b) => b,
        Err(e) => return e.into(),
    };
    let execution_requests = match encoded_requests_to_ssz(&bundle.requests) {
        Ok(r) => r,
        Err(e) => return e.into(),
    };
    let json = JsonExecutionPayload::from_block(bundle.block, bundle.block_access_list);
    let payload = match json_to_execution_payload_v4(&json) {
        Ok(p) => p,
        Err(e) => return e.into(),
    };
    add_no_store(
        SszBody(GetPayloadResponseV6 {
            execution_payload: payload,
            block_value,
            blobs_bundle,
            should_override_builder: false,
            execution_requests,
        })
        .into_response(),
    )
}

fn ssz_status_response(s: &PayloadStatus) -> Response {
    match json_payload_status_to_ssz(s) {
        Ok(ssz) => SszBody(ssz).into_response(),
        Err(e) => e.into(),
    }
}
