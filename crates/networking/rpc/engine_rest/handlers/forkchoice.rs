//! POST /{fork}/forkchoice — forkchoice update + optional payload build.

use axum::RequestExt;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
use ethrex_common::types::{ELASTICITY_MULTIPLIER, Fork, Withdrawal};
use ethrex_common::{Address, H256};
use tracing::{error, info};

use crate::engine::fork_choice::handle_forkchoice;
use crate::engine_rest::error::ProblemJson;
use crate::engine_rest::extractors::{decode_ssz, is_length_limit_error};
use crate::engine_rest::fork_path::ForkPath;
use crate::engine_rest::handlers::helpers::check_content_type;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::common::{
    ForkchoiceResponse, ForkchoiceState as SszForkchoiceState, PayloadId, PayloadStatus,
    PayloadStatusCode,
};
use crate::engine_rest::types::forkchoice_update::{
    AmsterdamForkchoiceUpdate, CancunForkchoiceUpdate, ParisForkchoiceUpdate,
    PragueForkchoiceUpdate, ShanghaiForkchoiceUpdate,
};
use crate::rpc::RpcApiContext;
use crate::types::fork_choice::{ForkChoiceState, PayloadAttributesV3, PayloadAttributesV4};
use crate::types::payload::PayloadValidationStatus;
use crate::utils::RpcErr;

/// Internal payload-attributes representation after fork-specific field mapping.
enum AttrsInternal {
    /// Paris / Shanghai / Cancun / Prague / Osaka → engine_forkchoiceUpdatedV{1..3}.
    V3(PayloadAttributesV3),
    /// Amsterdam → engine_forkchoiceUpdatedV4.
    V4(PayloadAttributesV4),
}

pub async fn forkchoice_update(
    ForkPath(fork): ForkPath,
    State(ctx): State<RpcApiContext>,
    req: axum::extract::Request,
) -> Response {
    if let Err(p) = check_content_type(req.headers()) {
        return p.into_response();
    }
    // `with_limited_body()` honours the DefaultBodyLimit middleware. Reading via
    // `req.into_body()` would bypass the cap and let an authenticated caller
    // buffer arbitrarily-large bodies in memory.
    let body = match axum::body::to_bytes(req.with_limited_body().into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            if is_length_limit_error(&e) {
                return ProblemJson::payload_too_large("request body exceeds configured limit")
                    .into_response();
            }
            return ProblemJson::bad_request(&format!("failed to read body: {e}")).into_response();
        }
    };

    match fork {
        Fork::Paris => {
            let update = match decode_ssz::<ParisForkchoiceUpdate>(&body) {
                Ok(u) => u,
                Err(p) => return p.into_response(),
            };
            let attrs = update
                .payload_attributes
                .map(|a| AttrsInternal::V3(paris_to_v3(a)));
            run_forkchoice(update.state, attrs, ctx, 1).await
        }
        Fork::Shanghai => {
            let update = match decode_ssz::<ShanghaiForkchoiceUpdate>(&body) {
                Ok(u) => u,
                Err(p) => return p.into_response(),
            };
            let attrs = update
                .payload_attributes
                .map(|a| AttrsInternal::V3(shanghai_to_v3(a)));
            run_forkchoice(update.state, attrs, ctx, 2).await
        }
        Fork::Cancun => {
            let update = match decode_ssz::<CancunForkchoiceUpdate>(&body) {
                Ok(u) => u,
                Err(p) => return p.into_response(),
            };
            let attrs = update
                .payload_attributes
                .map(|a| AttrsInternal::V3(cancun_to_v3(a)));
            run_forkchoice(update.state, attrs, ctx, 3).await
        }
        Fork::Prague => {
            let update = match decode_ssz::<PragueForkchoiceUpdate>(&body) {
                Ok(u) => u,
                Err(p) => return p.into_response(),
            };
            let attrs = update
                .payload_attributes
                .map(|a| AttrsInternal::V3(prague_to_v3(a)));
            // Prague uses forkchoiceUpdatedV3 semantics in the JSON-RPC layer.
            run_forkchoice(update.state, attrs, ctx, 3).await
        }
        Fork::Osaka => {
            // Osaka uses the same payload shape as Prague.
            let update = match decode_ssz::<PragueForkchoiceUpdate>(&body) {
                Ok(u) => u,
                Err(p) => return p.into_response(),
            };
            let attrs = update
                .payload_attributes
                .map(|a| AttrsInternal::V3(prague_to_v3(a)));
            run_forkchoice(update.state, attrs, ctx, 3).await
        }
        Fork::Amsterdam => {
            let update = match decode_ssz::<AmsterdamForkchoiceUpdate>(&body) {
                Ok(u) => u,
                Err(p) => return p.into_response(),
            };
            let attrs = update
                .payload_attributes
                .map(|a| AttrsInternal::V4(amsterdam_to_v4(a)));
            run_forkchoice(update.state, attrs, ctx, 4).await
        }
        // Unreachable: ForkPath's parse_fork_segment rejects all non-spec forks
        // with 400 before the handler runs.
        _ => unreachable!("ForkPath extractor restricts to the 6 spec forks"),
    }
}

// ── Fork-specific PayloadAttributes → internal conversion ────────────────────

fn withdrawals_from_ssz(ws: &[crate::engine_rest::types::shanghai::Withdrawal]) -> Vec<Withdrawal> {
    ws.iter()
        .map(|w| Withdrawal {
            index: w.index,
            validator_index: w.validator_index,
            address: Address::from(w.address.0),
            amount: w.amount,
        })
        .collect()
}

fn paris_to_v3(a: crate::engine_rest::types::paris::PayloadAttributes) -> PayloadAttributesV3 {
    PayloadAttributesV3 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from(a.suggested_fee_recipient.0),
        withdrawals: None,
        parent_beacon_block_root: None,
    }
}

fn shanghai_to_v3(
    a: crate::engine_rest::types::shanghai::PayloadAttributes,
) -> PayloadAttributesV3 {
    PayloadAttributesV3 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from(a.suggested_fee_recipient.0),
        withdrawals: Some(withdrawals_from_ssz(&a.withdrawals)),
        parent_beacon_block_root: None,
    }
}

fn cancun_to_v3(a: crate::engine_rest::types::cancun::PayloadAttributes) -> PayloadAttributesV3 {
    PayloadAttributesV3 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from(a.suggested_fee_recipient.0),
        withdrawals: Some(withdrawals_from_ssz(&a.withdrawals)),
        parent_beacon_block_root: Some(H256::from(a.parent_beacon_block_root)),
    }
}

fn prague_to_v3(a: crate::engine_rest::types::prague::PayloadAttributes) -> PayloadAttributesV3 {
    PayloadAttributesV3 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from(a.suggested_fee_recipient.0),
        withdrawals: Some(withdrawals_from_ssz(&a.withdrawals)),
        parent_beacon_block_root: Some(H256::from(a.parent_beacon_block_root)),
    }
}

fn amsterdam_to_v4(
    a: crate::engine_rest::types::amsterdam::PayloadAttributes,
) -> PayloadAttributesV4 {
    // `custody_columns` is decoded for spec compliance; not forwarded to the
    // payload builder (PeerDAS execution not yet landed in ethrex).
    // `slot_number` is not present in Amsterdam SSZ PayloadAttributes.
    // The V4 payload builder needs it for the payload-id hash; default to 0.
    PayloadAttributesV4 {
        timestamp: a.timestamp,
        prev_randao: H256::from(a.prev_randao),
        suggested_fee_recipient: Address::from(a.suggested_fee_recipient.0),
        withdrawals: Some(withdrawals_from_ssz(&a.withdrawals)),
        parent_beacon_block_root: Some(H256::from(a.parent_beacon_block_root)),
        slot_number: 0,
    }
}

// ── RpcErr → ProblemJson mapping ──────────────────────────────────────────────
//
// `handle_forkchoice` returns CL-driven conditions (unknown head during sync,
// too-deep reorg, attrs validation failures) as `RpcErr` variants that the
// JSON-RPC path translates into engine-API error codes (-38002/-38003/-38005/
// -38006). These are not server faults, so the REST path must not collapse them
// to HTTP 500 — that breaks CL retry/diagnostic logic.

fn rpc_err_to_problem(err: RpcErr) -> ProblemJson {
    match err {
        RpcErr::InvalidForkChoiceState(msg) => {
            ProblemJson::unprocessable_entity(&format!("invalid forkchoice state: {msg}"))
        }
        RpcErr::InvalidPayloadAttributes(msg) => {
            ProblemJson::unprocessable_entity(&format!("invalid payload attributes: {msg}"))
        }
        RpcErr::TooDeepReorg(msg) => {
            ProblemJson::conflict(&format!("reorg deeper than allowed: {msg}"))
        }
        RpcErr::UnsupportedFork(msg) => {
            ProblemJson::bad_request(&format!("unsupported fork: {msg}"))
        }
        RpcErr::BadParams(msg) | RpcErr::WrongParam(msg) | RpcErr::MissingParam(msg) => {
            ProblemJson::bad_request(&msg)
        }
        RpcErr::Internal(msg) => ProblemJson::internal(&format!("forkchoice failed: {msg}")),
        other => ProblemJson::internal(&format!("forkchoice failed: {other}")),
    }
}

// ── Core forkchoice execution ────────────────────────────────────────────────

async fn run_forkchoice(
    state: SszForkchoiceState,
    attrs: Option<AttrsInternal>,
    ctx: RpcApiContext,
    version: usize,
) -> Response {
    let internal_state = ForkChoiceState {
        head_block_hash: H256::from(state.head_block_hash),
        safe_block_hash: H256::from(state.safe_block_hash),
        finalized_block_hash: H256::from(state.finalized_block_hash),
    };

    let (head_header, mut response) =
        match handle_forkchoice(&internal_state, ctx.clone(), version).await {
            Ok(r) => r,
            Err(err) => return rpc_err_to_problem(err).into_response(),
        };

    // If payload_attributes were provided and we have a known head, kick off a build.
    if let (Some(attrs_internal), Some(_head)) = (attrs, head_header) {
        let build_result = match attrs_internal {
            AttrsInternal::V3(v3) => {
                build_payload_v3(&v3, ctx, &internal_state, version as u8).await
            }
            AttrsInternal::V4(v4) => build_payload_v4(&v4, ctx, &internal_state).await,
        };
        match build_result {
            Ok(payload_id) => response.set_id(payload_id),
            Err(err) => return rpc_err_to_problem(err).into_response(),
        }
    }

    // Map internal ForkChoiceResponse → SSZ ForkchoiceResponse.
    let status_code = match response.payload_status.status {
        PayloadValidationStatus::Valid => PayloadStatusCode::Valid,
        PayloadValidationStatus::Invalid => PayloadStatusCode::Invalid,
        PayloadValidationStatus::Syncing => PayloadStatusCode::Syncing,
        PayloadValidationStatus::Accepted => {
            // Spec: /forkchoice MUST NOT return ACCEPTED.
            error!("/forkchoice returned ACCEPTED — server bug");
            return ProblemJson::internal(
                "internal layer returned ACCEPTED, forbidden on /forkchoice",
            )
            .into_response();
        }
    };

    let ssz_status = PayloadStatus {
        status: status_code as u8,
        latest_valid_hash: response.payload_status.latest_valid_hash.map(|h| h.0),
        validation_error: response.payload_status.validation_error,
    };
    let payload_id = response.payload_id.map(PayloadId::from_u64);

    SszBody(ForkchoiceResponse {
        payload_status: ssz_status,
        payload_id,
    })
    .into_response()
}

// ── Payload build helpers ─────────────────────────────────────────────────────

async fn build_payload_v3(
    attrs: &PayloadAttributesV3,
    ctx: RpcApiContext,
    fork_choice_state: &ForkChoiceState,
    version: u8,
) -> Result<u64, RpcErr> {
    let args = BuildPayloadArgs {
        parent: fork_choice_state.head_block_hash,
        timestamp: attrs.timestamp,
        fee_recipient: attrs.suggested_fee_recipient,
        random: attrs.prev_randao,
        withdrawals: attrs.withdrawals.clone(),
        beacon_root: attrs.parent_beacon_block_root,
        slot_number: None,
        version,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: ctx.gas_ceil,
    };
    let payload_id = args
        .id()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    info!(
        id = payload_id,
        version, "engine REST forkchoice: creating payload"
    );
    let payload = match create_payload(&args, &ctx.storage, ctx.node_data.extra_data) {
        Ok(p) => p,
        Err(ChainError::EvmError(error)) => return Err(error.into()),
        Err(error) => return Err(RpcErr::Internal(error.to_string())),
    };
    ctx.blockchain
        .initiate_payload_build(payload, payload_id)
        .await;
    Ok(payload_id)
}

async fn build_payload_v4(
    attrs: &PayloadAttributesV4,
    ctx: RpcApiContext,
    fork_choice_state: &ForkChoiceState,
) -> Result<u64, RpcErr> {
    let args = BuildPayloadArgs {
        parent: fork_choice_state.head_block_hash,
        timestamp: attrs.timestamp,
        fee_recipient: attrs.suggested_fee_recipient,
        random: attrs.prev_randao,
        withdrawals: attrs.withdrawals.clone(),
        beacon_root: attrs.parent_beacon_block_root,
        slot_number: Some(attrs.slot_number),
        version: 4,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: ctx.gas_ceil,
    };
    let payload_id = args
        .id()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    info!(
        id = payload_id,
        slot = attrs.slot_number,
        "engine REST forkchoice V4: creating payload"
    );
    let payload = match create_payload(&args, &ctx.storage, ctx.node_data.extra_data) {
        Ok(p) => p,
        Err(ChainError::EvmError(error)) => return Err(error.into()),
        Err(error) => return Err(RpcErr::Internal(error.to_string())),
    };
    ctx.blockchain
        .initiate_payload_build(payload, payload_id)
        .await;
    Ok(payload_id)
}
