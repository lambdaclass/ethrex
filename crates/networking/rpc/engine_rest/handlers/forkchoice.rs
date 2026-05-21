//! POST /engine/v{1..4}/forkchoice.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use ethrex_common::{Address, H256};

use crate::engine::fork_choice::{
    build_payload, build_payload_v4, handle_forkchoice, validate_attributes_v1,
    validate_attributes_v2, validate_attributes_v2_pre_shanghai, validate_attributes_v3,
    validate_attributes_v4,
};
use crate::engine_rest::conversions::{json_payload_status_to_ssz, ssz_withdrawals_to_vec};
use crate::engine_rest::error::{EngineError, EngineRestError, classify_rpc_err};
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::common::{
    ForkchoiceStateV1 as SszForkchoiceState, ForkchoiceUpdatedResponseV1, ssz_none, ssz_some,
};
use crate::engine_rest::types::forkchoice::{
    ForkchoiceUpdatedV1Request, ForkchoiceUpdatedV2Request, ForkchoiceUpdatedV3Request,
    ForkchoiceUpdatedV4Request,
};
use crate::engine_rest::types::payload_attributes::{
    PayloadAttributesV1, PayloadAttributesV2, PayloadAttributesV3, PayloadAttributesV4,
};
use crate::rpc::RpcApiContext;
use crate::types::fork_choice::{
    ForkChoiceState, PayloadAttributesV3 as JsonPayloadAttributesV3,
    PayloadAttributesV4 as JsonPayloadAttributesV4,
};
use crate::types::payload::PayloadStatus;

fn to_fork_choice_state(s: &SszForkchoiceState) -> ForkChoiceState {
    ForkChoiceState {
        head_block_hash: H256::from(s.head_block_hash),
        safe_block_hash: H256::from(s.safe_block_hash),
        finalized_block_hash: H256::from(s.finalized_block_hash),
    }
}

fn payload_attributes_v1_to_internal(a: &PayloadAttributesV1) -> JsonPayloadAttributesV3 {
    JsonPayloadAttributesV3 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from_slice(&a.suggested_fee_recipient),
        withdrawals: None,
        parent_beacon_block_root: None,
    }
}

fn payload_attributes_v2_to_internal(a: &PayloadAttributesV2) -> JsonPayloadAttributesV3 {
    JsonPayloadAttributesV3 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from_slice(&a.suggested_fee_recipient),
        withdrawals: Some(ssz_withdrawals_to_vec(&a.withdrawals)),
        parent_beacon_block_root: None,
    }
}

fn payload_attributes_v3_to_internal(a: &PayloadAttributesV3) -> JsonPayloadAttributesV3 {
    JsonPayloadAttributesV3 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from_slice(&a.suggested_fee_recipient),
        withdrawals: Some(ssz_withdrawals_to_vec(&a.withdrawals)),
        parent_beacon_block_root: Some(H256::from(a.parent_beacon_block_root)),
    }
}

fn payload_attributes_v4_to_internal(a: &PayloadAttributesV4) -> JsonPayloadAttributesV4 {
    JsonPayloadAttributesV4 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from_slice(&a.suggested_fee_recipient),
        withdrawals: Some(ssz_withdrawals_to_vec(&a.withdrawals)),
        parent_beacon_block_root: Some(H256::from(a.parent_beacon_block_root)),
        slot_number: a.slot_number,
        target_gas_limit: Some(a.target_gas_limit),
    }
}

fn response_to_ssz(
    payload_status: PayloadStatus,
    payload_id: Option<u64>,
) -> Result<ForkchoiceUpdatedResponseV1, EngineRestError> {
    let ssz_status = json_payload_status_to_ssz(&payload_status)?;
    let id_list = match payload_id {
        Some(id) => ssz_some(id.to_be_bytes()),
        None => ssz_none(),
    };
    Ok(ForkchoiceUpdatedResponseV1 {
        payload_status: ssz_status,
        payload_id: id_list,
    })
}

pub async fn forkchoice_v1(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<ForkchoiceUpdatedV1Request>,
) -> Response {
    let fcs = to_fork_choice_state(&req.forkchoice_state);
    let attrs: Option<JsonPayloadAttributesV3> = req
        .payload_attributes
        .first()
        .map(payload_attributes_v1_to_internal);

    let (head_block_opt, mut response) = match handle_forkchoice(&fcs, ctx.clone(), 1).await {
        Ok(r) => r,
        Err(e) => return classify_rpc_err(e),
    };

    if let (Some(head_block), Some(attrs)) = (head_block_opt, attrs.as_ref()) {
        if ctx
            .storage
            .get_chain_config()
            .is_cancun_activated(attrs.timestamp)
        {
            return EngineError::unprocessable("forkChoiceV1 used to build Cancun payload");
        }
        if let Err(e) = validate_attributes_v1(attrs, &head_block) {
            return classify_rpc_err(e);
        }
        match build_payload(attrs, ctx, &fcs, 1).await {
            Ok(id) => response.set_id(id),
            Err(e) => return classify_rpc_err(e),
        }
    }

    finalize(response)
}

pub async fn forkchoice_v2(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<ForkchoiceUpdatedV2Request>,
) -> Response {
    let fcs = to_fork_choice_state(&req.forkchoice_state);
    let attrs: Option<JsonPayloadAttributesV3> = req
        .payload_attributes
        .first()
        .map(payload_attributes_v2_to_internal);

    let (head_block_opt, mut response) = match handle_forkchoice(&fcs, ctx.clone(), 2).await {
        Ok(r) => r,
        Err(e) => return classify_rpc_err(e),
    };

    if let (Some(head_block), Some(attrs)) = (head_block_opt, attrs.as_ref()) {
        let chain_config = ctx.storage.get_chain_config();
        if chain_config.is_cancun_activated(attrs.timestamp) {
            return EngineError::unprocessable("forkChoiceV2 used to build Cancun payload");
        }
        let validation = if chain_config.is_shanghai_activated(attrs.timestamp) {
            validate_attributes_v2(attrs, &head_block)
        } else {
            validate_attributes_v2_pre_shanghai(attrs, &head_block)
        };
        if let Err(e) = validation {
            return classify_rpc_err(e);
        }
        match build_payload(attrs, ctx, &fcs, 2).await {
            Ok(id) => response.set_id(id),
            Err(e) => return classify_rpc_err(e),
        }
    }

    finalize(response)
}

pub async fn forkchoice_v3(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<ForkchoiceUpdatedV3Request>,
) -> Response {
    let fcs = to_fork_choice_state(&req.forkchoice_state);
    let attrs: Option<JsonPayloadAttributesV3> = req
        .payload_attributes
        .first()
        .map(payload_attributes_v3_to_internal);

    let (head_block_opt, mut response) = match handle_forkchoice(&fcs, ctx.clone(), 3).await {
        Ok(r) => r,
        Err(e) => return classify_rpc_err(e),
    };

    if let (Some(head_block), Some(attrs)) = (head_block_opt, attrs.as_ref()) {
        if let Err(e) = validate_attributes_v3(attrs, &head_block, &ctx) {
            return classify_rpc_err(e);
        }
        match build_payload(attrs, ctx, &fcs, 3).await {
            Ok(id) => response.set_id(id),
            Err(e) => return classify_rpc_err(e),
        }
    }

    finalize(response)
}

pub async fn forkchoice_v4(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<ForkchoiceUpdatedV4Request>,
) -> Response {
    let fcs = to_fork_choice_state(&req.forkchoice_state);
    let attrs: Option<JsonPayloadAttributesV4> = req
        .payload_attributes
        .first()
        .map(payload_attributes_v4_to_internal);

    let (head_block_opt, mut response) = match handle_forkchoice(&fcs, ctx.clone(), 4).await {
        Ok(r) => r,
        Err(e) => return classify_rpc_err(e),
    };

    if let (Some(head_block), Some(attrs)) = (head_block_opt, attrs.as_ref()) {
        if let Err(e) = validate_attributes_v4(attrs, &head_block, &ctx) {
            return classify_rpc_err(e);
        }
        match build_payload_v4(attrs, ctx, &fcs).await {
            Ok(id) => response.set_id(id),
            Err(e) => return classify_rpc_err(e),
        }
    }

    finalize(response)
}

fn finalize(response: crate::types::fork_choice::ForkChoiceResponse) -> Response {
    match response_to_ssz(response.payload_status, response.payload_id) {
        Ok(ssz) => SszBody(ssz).into_response(),
        Err(e) => e.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssz_v4(target_gas_limit: u64) -> PayloadAttributesV4 {
        PayloadAttributesV4 {
            timestamp: 100,
            prev_randao: [1u8; 32],
            suggested_fee_recipient: [2u8; 20],
            withdrawals: Vec::new().try_into().unwrap(),
            parent_beacon_block_root: [3u8; 32],
            slot_number: 42,
            target_gas_limit,
        }
    }

    // The SSZ wire is a non-nullable u64; we pass `Some(value)` through. The
    // 0-sentinel collapse to `None` happens in `build_payload_v4` per EIP-7783
    // so JSON-RPC and SSZ share one source of truth.
    #[test]
    fn target_gas_limit_round_trips_as_some() {
        assert_eq!(
            payload_attributes_v4_to_internal(&ssz_v4(0)).target_gas_limit,
            Some(0)
        );
        assert_eq!(
            payload_attributes_v4_to_internal(&ssz_v4(36_000_000)).target_gas_limit,
            Some(36_000_000)
        );
    }
}
