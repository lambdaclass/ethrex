use ethrex_blockchain::{
    error::{ChainError, InvalidForkChoice},
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::types::{BlockHeader, ELASTICITY_MULTIPLIER};
use ethrex_p2p::sync::SyncMode;
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    subscription_manager::SubscriptionManagerProtocol,
    types::{
        fork_choice::{
            ForkChoiceResponse, ForkChoiceState, PayloadAttributesV3, PayloadAttributesV4,
        },
        payload::PayloadStatus,
    },
    utils::RpcErr,
    utils::RpcRequest,
};

/// Parse a `custodyColumns` RPC param (16-byte big-endian hex string or null/absent).
///
/// Spec: `DATA|null` where DATA is a 0x-prefixed 16-byte hex string.
/// Returns `Ok(None)` for JSON null or absent param.
/// Returns `Err(RpcErr::BadParams)` when the string is present but not exactly 16 bytes.
pub(crate) fn parse_custody_columns(value: &Value) -> Result<Option<u128>, RpcErr> {
    if value.is_null() {
        return Ok(None);
    }
    let hex_str = value
        .as_str()
        .ok_or_else(|| RpcErr::BadParams("custodyColumns must be a hex string or null".into()))?;
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(stripped)
        .map_err(|_| RpcErr::BadParams("custodyColumns: invalid hex".into()))?;
    if bytes.len() != 16 {
        return Err(RpcErr::BadParams(format!(
            "custodyColumns must be 16 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes);
    Ok(Some(u128::from_be_bytes(arr)))
}

/// Apply a custody column update received via `engine_forkchoiceUpdatedV4`.
///
/// Lock discipline: each mempool accessor takes its own short-lived guard;
/// no guard is held across the set+prune sequence.
///
/// Broadcast wiring note: direct p2p broadcast of updated availability is not
/// reachable from `RpcApiContext` (the context exposes a `PeerHandler` but not
/// a channel to push availability advertisements). The set and prune steps are
/// applied immediately; the p2p layer will pick up the updated `custody_columns`
/// value on its next announce/flush cycle via `mempool.get_custody_columns()`.
pub(crate) fn apply_custody_update(context: &RpcApiContext, custody_columns: Option<u128>) {
    let Some(new) = custody_columns else {
        // null / absent param — no custody change.
        return;
    };

    let mempool = &context.blockchain.mempool;

    // Read previous value with a short guard, then drop it before any further
    // mempool access (RwLock is not reentrant on std).
    let prev = match mempool.get_custody_columns() {
        Ok(v) => v,
        Err(e) => {
            warn!("apply_custody_update: failed to read custody_columns: {e}");
            return;
        }
    };

    if prev == new {
        // Identical — no-op.
        return;
    }

    let expanded = new & !prev; // bits added
    let contracted = prev & !new; // bits removed

    // Update the custody set; the p2p layer reads it on its next announce/flush
    // cycle. Column-level cell pruning on contraction is intentionally NOT done
    // here: retaining the extra cells is harmless and keeps them available to
    // serving peers (so no availability-fault window), and the periodic mempool
    // sweep still drops all cells for txs that leave the pool. See PLAN follow-ups
    // for column-level pruning + broadcast-before-prune ordering.
    if let Err(e) = mempool.set_custody_columns(new) {
        warn!("apply_custody_update: failed to set custody_columns: {e}");
        return;
    }

    if expanded != 0 {
        // set_custody_columns bumped the mempool custody generation; the p2p
        // sweep re-samples already-pending blob txs for the new delta columns
        // (EIP MUST). Inert while sampling is off.
        debug!(
            expanded_mask = %format!("{expanded:#034x}"),
            "custody columns expanded; pending blob txs will be re-sampled for new columns",
        );
    }
    if contracted != 0 {
        debug!(
            contracted_mask = %format!("{contracted:#034x}"),
            "custody columns contracted; extra cells retained (column-level pruning deferred)",
        );
    }
}

#[derive(Debug)]
pub struct ForkChoiceUpdatedV1 {
    pub fork_choice_state: ForkChoiceState,
    pub payload_attributes: Option<PayloadAttributesV3>,
}

impl RpcHandler for ForkChoiceUpdatedV1 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let (fork_choice_state, payload_attributes) = parse(params, false)?;
        Ok(ForkChoiceUpdatedV1 {
            fork_choice_state,
            payload_attributes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let (head_block_opt, mut response) =
            handle_forkchoice(&self.fork_choice_state, context.clone(), 1).await?;
        if let (Some(head_block), Some(attributes)) = (head_block_opt, &self.payload_attributes) {
            let chain_config = context.storage.get_chain_config();
            if chain_config.is_cancun_activated(attributes.timestamp) {
                return Err(RpcErr::UnsupportedFork(
                    "forkChoiceV1 used to build Cancun payload".to_string(),
                ));
            }
            validate_attributes_v1(attributes, &head_block)?;
            let payload_id = build_payload(attributes, context, &self.fork_choice_state, 1).await?;
            response.set_id(payload_id);
        }
        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[derive(Debug)]
pub struct ForkChoiceUpdatedV2 {
    pub fork_choice_state: ForkChoiceState,
    pub payload_attributes: Option<PayloadAttributesV3>,
}

impl RpcHandler for ForkChoiceUpdatedV2 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let (fork_choice_state, payload_attributes) = parse(params, false)?;
        Ok(ForkChoiceUpdatedV2 {
            fork_choice_state,
            payload_attributes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let (head_block_opt, mut response) =
            handle_forkchoice(&self.fork_choice_state, context.clone(), 2).await?;
        if let (Some(head_block), Some(attributes)) = (head_block_opt, &self.payload_attributes) {
            let chain_config = context.storage.get_chain_config();
            if chain_config.is_cancun_activated(attributes.timestamp) {
                return Err(RpcErr::UnsupportedFork(
                    "forkChoiceV2 used to build Cancun payload".to_string(),
                ));
            } else if chain_config.is_shanghai_activated(attributes.timestamp) {
                validate_attributes_v2(attributes, &head_block)?;
            } else {
                validate_attributes_v2_pre_shanghai(attributes, &head_block)?;
            }
            let payload_id = build_payload(attributes, context, &self.fork_choice_state, 2).await?;
            response.set_id(payload_id);
        }
        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[derive(Debug)]
pub struct ForkChoiceUpdatedV3 {
    pub fork_choice_state: ForkChoiceState,
    pub payload_attributes: Option<PayloadAttributesV3>,
}

impl From<ForkChoiceUpdatedV3> for RpcRequest {
    fn from(val: ForkChoiceUpdatedV3) -> Self {
        RpcRequest {
            method: "engine_forkchoiceUpdatedV3".to_string(),
            params: Some(vec![
                serde_json::json!(val.fork_choice_state),
                serde_json::json!(val.payload_attributes),
            ]),
            ..Default::default()
        }
    }
}

impl RpcHandler for ForkChoiceUpdatedV3 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let (fork_choice_state, payload_attributes) = parse(params, true)?;
        Ok(ForkChoiceUpdatedV3 {
            fork_choice_state,
            payload_attributes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let (head_block_opt, mut response) =
            handle_forkchoice(&self.fork_choice_state, context.clone(), 3).await?;
        if let (Some(head_block), Some(attributes)) = (head_block_opt, &self.payload_attributes) {
            validate_attributes_v3(attributes, &head_block, &context)?;
            let payload_id = build_payload(attributes, context, &self.fork_choice_state, 3).await?;
            response.set_id(payload_id);
        }
        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[derive(Debug)]
pub struct ForkChoiceUpdatedV4 {
    pub fork_choice_state: ForkChoiceState,
    pub payload_attributes: Option<PayloadAttributesV4>,
    /// Optional custody column bitmask from the 3rd Engine API parameter.
    /// `None` means the param was absent or null; `Some(mask)` triggers a
    /// custody column update in the mempool.
    pub custody_columns: Option<u128>,
}

impl From<ForkChoiceUpdatedV4> for RpcRequest {
    fn from(val: ForkChoiceUpdatedV4) -> Self {
        let custody_hex = val
            .custody_columns
            .map(|m| format!("0x{}", hex::encode(m.to_be_bytes())))
            .map(Value::String)
            .unwrap_or(Value::Null);
        RpcRequest {
            method: "engine_forkchoiceUpdatedV4".to_string(),
            params: Some(vec![
                serde_json::json!(val.fork_choice_state),
                serde_json::json!(val.payload_attributes),
                custody_hex,
            ]),
            ..Default::default()
        }
    }
}

impl RpcHandler for ForkChoiceUpdatedV4 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let (fork_choice_state, payload_attributes, custody_columns) = parse_v4(params)?;
        Ok(ForkChoiceUpdatedV4 {
            fork_choice_state,
            payload_attributes,
            custody_columns,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let (head_block_opt, mut response) =
            handle_forkchoice(&self.fork_choice_state, context.clone(), 4).await?;
        apply_custody_update(&context, self.custody_columns);
        if let (Some(head_block), Some(attributes)) = (head_block_opt, &self.payload_attributes) {
            validate_attributes_v4(attributes, &head_block, &context)?;
            let payload_id = build_payload_v4(attributes, context, &self.fork_choice_state).await?;
            response.set_id(payload_id);
        }
        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

fn parse(
    params: &Option<Vec<Value>>,
    is_v3: bool,
) -> Result<(ForkChoiceState, Option<PayloadAttributesV3>), RpcErr> {
    let params = params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;

    if params.len() != 2 && params.len() != 1 {
        return Err(RpcErr::BadParams("Expected 2 or 1 params".to_owned()));
    }

    let forkchoice_state: ForkChoiceState = serde_json::from_value(params[0].clone())?;
    let mut payload_attributes: Option<PayloadAttributesV3> = None;
    if params.len() == 2 {
        // if there is an error when parsing (or the parameter is missing), set to None
        payload_attributes =
            match serde_json::from_value::<Option<PayloadAttributesV3>>(params[1].clone()) {
                Ok(attributes) => attributes,
                Err(error) => {
                    warn!("Could not parse payload attributes {}", error);
                    None
                }
            };
    }

    if payload_attributes
        .as_ref()
        .is_some_and(|attr| !is_v3 && attr.parent_beacon_block_root.is_some())
    {
        return Err(RpcErr::InvalidPayloadAttributes(
            "Attribute parent_beacon_block_root is non-null".to_string(),
        ));
    }
    Ok((forkchoice_state, payload_attributes))
}

async fn handle_forkchoice(
    fork_choice_state: &ForkChoiceState,
    context: RpcApiContext,
    version: usize,
) -> Result<(Option<BlockHeader>, ForkChoiceResponse), RpcErr> {
    let Some(syncer) = &context.syncer else {
        return Err(RpcErr::Internal(
            "Fork choice requested but syncer is not initialized".to_string(),
        ));
    };
    debug!(
        version = %format!("v{}", version),
        head = %format!("{:#x}", fork_choice_state.head_block_hash),
        safe = %format!("{:#x}", fork_choice_state.safe_block_hash),
        finalized = %format!("{:#x}", fork_choice_state.finalized_block_hash),
        "New fork choice update",
    );

    if let Some(latest_valid_hash) = context
        .storage
        .get_latest_valid_ancestor(fork_choice_state.head_block_hash)
        .await?
    {
        return Ok((
            None,
            ForkChoiceResponse::from(PayloadStatus::invalid_with(
                latest_valid_hash,
                InvalidForkChoice::InvalidAncestor(latest_valid_hash).to_string(),
            )),
        ));
    }

    // Check parent block hash in invalid_ancestors (if head block exists)
    if let Some(head_block) = context
        .storage
        .get_block_header_by_hash(fork_choice_state.head_block_hash)?
        && let Some(latest_valid_hash) = context
            .storage
            .get_latest_valid_ancestor(head_block.parent_hash)
            .await?
    {
        // Invalidate the child too
        context
            .storage
            .set_latest_valid_ancestor(head_block.hash(), latest_valid_hash)
            .await?;
        return Ok((
            None,
            ForkChoiceResponse::from(PayloadStatus::invalid_with(
                latest_valid_hash,
                InvalidForkChoice::InvalidAncestor(latest_valid_hash).to_string(),
            )),
        ));
    }

    // Ignore any FCU during snap-sync.
    // Processing the FCU while snap-syncing can result in reading inconsistent data
    // from the DB, and the later head update can overwrite changes made by the syncer
    // process, corrupting the forkchoice state (see #5547)
    if syncer.sync_mode() == SyncMode::Snap {
        syncer.sync_to_head(fork_choice_state.head_block_hash);
        return Ok((None, PayloadStatus::syncing().into()));
    }

    match apply_fork_choice(
        &context.storage,
        fork_choice_state.head_block_hash,
        fork_choice_state.safe_block_hash,
        fork_choice_state.finalized_block_hash,
    )
    .await
    {
        Ok(head) => {
            // Fork Choice was succesful, the node is up to date with the current chain
            context.blockchain.set_synced();
            // Remove included transactions from the mempool after we accept the fork choice
            // TODO(#797): The remove of transactions from the mempool could be incomplete (i.e. REORGS)
            match context.storage.get_block_by_hash(head.hash()).await {
                Ok(Some(block)) => {
                    // Remove executed transactions from mempool
                    context
                        .blockchain
                        .remove_block_transactions_from_pool(&block)?;
                    // Reset blob sub-pool against on-chain nonces (head-block
                    // pruning above misses stale blobs from non-head blocks).
                    // Best-effort housekeeping: a state-read failure here must
                    // not fail an otherwise-successful FCU, so log and continue
                    // rather than propagating. The next FCU re-runs the sweep.
                    if let Err(err) = context.blockchain.remove_stale_blob_txs(block.hash()).await {
                        warn!(
                            "Failed to prune stale blob txs from mempool after fork choice: {err}"
                        );
                    }
                    // Re-simulate pending frame txs (EIP-8141) whose validity may
                    // have changed because of this block, evicting any that no
                    // longer pass. This runs an EVM validation-prefix simulation
                    // per pending frame tx, so it is offloaded to the blocking
                    // pool to avoid stalling the async FCU worker. Best-effort
                    // housekeeping (local peer policy): a failure must not fail an
                    // otherwise-successful FCU, so log and continue. (Running it
                    // fully outside the FCU handler is a deferred follow-up.)
                    let blockchain = context.blockchain.clone();
                    match tokio::task::spawn_blocking(move || {
                        blockchain.revalidate_frame_txs_after_block(&block)
                    })
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => warn!(
                            "Failed to revalidate pending frame txs from mempool after fork choice: {err}"
                        ),
                        Err(err) => warn!(
                            "Frame-tx revalidation task failed to join after fork choice: {err}"
                        ),
                    }
                }
                Ok(None) => {
                    warn!(
                        "Couldn't get block by hash to remove transactions from the mempool. This is expected in a reconstruted network"
                    )
                }
                Err(_) => {
                    return Err(RpcErr::Internal(
                        "Failed to get block by hash to remove transactions from the mempool"
                            .to_string(),
                    ));
                }
            };

            // Notify all eth_subscribe("newHeads") subscribers.
            if let Some(ws) = &context.ws {
                let _ = ws.subscription_manager.new_head(head.clone());
            }

            Ok((
                Some(head),
                ForkChoiceResponse::from(PayloadStatus::valid_with_hash(
                    fork_choice_state.head_block_hash,
                )),
            ))
        }
        Err(forkchoice_error) => {
            let forkchoice_response = match forkchoice_error {
                InvalidForkChoice::NewHeadAlreadyCanonical => {
                    // execution-apis PR 786: when head references a VALID ancestor of
                    // the latest known finalized block, return VALID + null payloadId
                    // and MUST NOT begin a payload build process. We return `None` for
                    // the head header so the V3/V4 dispatch short-circuits the
                    // build_payload call.
                    context.blockchain.set_synced();
                    return Ok((
                        None,
                        ForkChoiceResponse::from(PayloadStatus::valid_with_hash(
                            fork_choice_state.head_block_hash,
                        )),
                    ));
                }
                InvalidForkChoice::Syncing => {
                    // Start sync
                    syncer.sync_to_head(fork_choice_state.head_block_hash);
                    ForkChoiceResponse::from(PayloadStatus::syncing())
                }
                // TODO(#5564): handle arbitrary reorgs
                InvalidForkChoice::StateNotReachable => {
                    // We can't reach the head's state from our DB (the nearest
                    // link block has pruned or not-yet-executed state). Kick off
                    // a sync toward the head instead of reporting SYNCING while
                    // sitting idle, which wedges the node: the CL keeps resending
                    // FCUs we keep ignoring and we never make progress.
                    // sync_to_head is idempotent (only starts a cycle if the
                    // syncer is inactive) and mode-agnostic, so this is safe for
                    // both full and snap clients.
                    syncer.sync_to_head(fork_choice_state.head_block_hash);
                    ForkChoiceResponse::from(PayloadStatus::syncing())
                }
                InvalidForkChoice::Disconnected(_, _) | InvalidForkChoice::ElementNotFound(_) => {
                    warn!("Invalid fork choice state. Reason: {:?}", forkchoice_error);
                    return Err(RpcErr::InvalidForkChoiceState(forkchoice_error.to_string()));
                }
                InvalidForkChoice::TooDeepReorg { .. } => {
                    warn!("Rejecting fork choice update. Reason: {forkchoice_error}");
                    return Err(RpcErr::TooDeepReorg(forkchoice_error.to_string()));
                }
                InvalidForkChoice::InvalidAncestor(last_valid_hash) => {
                    ForkChoiceResponse::from(PayloadStatus::invalid_with(
                        last_valid_hash,
                        InvalidForkChoice::InvalidAncestor(last_valid_hash).to_string(),
                    ))
                }
                reason => {
                    warn!(
                        "Invalid fork choice payload. Reason: {}",
                        reason.to_string()
                    );
                    let latest_valid_hash = context
                        .storage
                        .get_latest_canonical_block_hash()
                        .await?
                        .ok_or(RpcErr::Internal(
                            "Missing latest canonical block".to_owned(),
                        ))?;
                    ForkChoiceResponse::from(PayloadStatus::invalid_with(
                        latest_valid_hash,
                        reason.to_string(),
                    ))
                }
            };
            Ok((None, forkchoice_response))
        }
    }
}

fn validate_attributes_v1(
    attributes: &PayloadAttributesV3,
    head_block: &BlockHeader,
) -> Result<(), RpcErr> {
    if attributes.withdrawals.is_some() {
        return Err(RpcErr::WrongParam("withdrawals".to_string()));
    }
    validate_timestamp(attributes, head_block)
}

fn validate_attributes_v2(
    attributes: &PayloadAttributesV3,
    head_block: &BlockHeader,
) -> Result<(), RpcErr> {
    if attributes.withdrawals.is_none() {
        return Err(RpcErr::InvalidPayloadAttributes("withdrawals".to_string()));
    }
    validate_timestamp(attributes, head_block)
}

fn validate_attributes_v2_pre_shanghai(
    attributes: &PayloadAttributesV3,
    head_block: &BlockHeader,
) -> Result<(), RpcErr> {
    if attributes.withdrawals.is_some() {
        return Err(RpcErr::InvalidPayloadAttributes("withdrawals".to_string()));
    }
    validate_timestamp(attributes, head_block)
}

fn validate_attributes_v3(
    attributes: &PayloadAttributesV3,
    head_block: &BlockHeader,
    context: &RpcApiContext,
) -> Result<(), RpcErr> {
    let chain_config = context.storage.get_chain_config();
    // Specification indicates this order of validations:
    // https://github.com/ethereum/execution-apis/blob/main/src/engine/cancun.md#specification-1
    if attributes.withdrawals.is_none() {
        return Err(RpcErr::InvalidPayloadAttributes("withdrawals".to_string()));
    }
    if attributes.parent_beacon_block_root.is_none() {
        return Err(RpcErr::InvalidPayloadAttributes(
            "Attribute parent_beacon_block_root is null".to_string(),
        ));
    }
    if chain_config.is_amsterdam_activated(attributes.timestamp) {
        return Err(RpcErr::UnsupportedFork(
            "forkChoiceV3 used to build Amsterdam payload".to_string(),
        ));
    }
    if !chain_config.is_cancun_activated(attributes.timestamp) {
        return Err(RpcErr::UnsupportedFork(
            "forkChoiceV3 used to build pre-Cancun payload".to_string(),
        ));
    }
    validate_timestamp(attributes, head_block)
}

fn validate_timestamp(
    attributes: &PayloadAttributesV3,
    head_block: &BlockHeader,
) -> Result<(), RpcErr> {
    if attributes.timestamp <= head_block.timestamp {
        return Err(RpcErr::InvalidPayloadAttributes(
            "invalid timestamp".to_string(),
        ));
    }
    Ok(())
}

async fn build_payload(
    attributes: &PayloadAttributesV3,
    context: RpcApiContext,
    fork_choice_state: &ForkChoiceState,
    version: u8,
) -> Result<u64, RpcErr> {
    let args = BuildPayloadArgs {
        parent: fork_choice_state.head_block_hash,
        timestamp: attributes.timestamp,
        fee_recipient: attributes.suggested_fee_recipient,
        random: attributes.prev_randao,
        withdrawals: attributes.withdrawals.clone(),
        beacon_root: attributes.parent_beacon_block_root,
        slot_number: None,
        version,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: context.gas_ceil,
    };
    let payload_id = args
        .id()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;

    info!(
        id = payload_id,
        "Fork choice updated includes payload attributes. Creating a new payload"
    );
    let payload = match create_payload(&args, &context.storage, context.node_data.extra_data) {
        Ok(payload) => payload,
        Err(ChainError::EvmError(error)) => return Err(error.into()),
        // Parent block is guaranteed to be present at this point,
        // so the only errors that may be returned are internal storage errors
        Err(error) => return Err(RpcErr::Internal(error.to_string())),
    };
    context
        .blockchain
        .initiate_payload_build(payload, payload_id)
        .await;
    Ok(payload_id)
}

pub(crate) fn parse_v4(
    params: &Option<Vec<Value>>,
) -> Result<(ForkChoiceState, Option<PayloadAttributesV4>, Option<u128>), RpcErr> {
    let params = params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;

    if params.len() > 3 || params.is_empty() {
        return Err(RpcErr::BadParams("Expected 1, 2, or 3 params".to_owned()));
    }

    let forkchoice_state: ForkChoiceState = serde_json::from_value(params[0].clone())?;

    // execution-apis#796: V4 attributes are validated strictly. A present but
    // malformed object (e.g. missing the required targetGasLimit) is rejected
    // rather than silently ignored; an absent/null object yields no attributes.
    let payload_attributes = if params.len() >= 2 {
        serde_json::from_value::<Option<PayloadAttributesV4>>(params[1].clone()).map_err(
            |error| {
                RpcErr::InvalidPayloadAttributes(format!("invalid V4 payload attributes: {error}"))
            },
        )?
    } else {
        None
    };

    let custody_columns = if params.len() == 3 {
        parse_custody_columns(&params[2])?
    } else {
        None
    };

    Ok((forkchoice_state, payload_attributes, custody_columns))
}

fn validate_attributes_v4(
    attributes: &PayloadAttributesV4,
    head_block: &BlockHeader,
    context: &RpcApiContext,
) -> Result<(), RpcErr> {
    // Similar validation to V3
    let chain_config = context.storage.get_chain_config();
    if !chain_config.is_amsterdam_activated(attributes.timestamp) {
        return Err(RpcErr::InvalidPayloadAttributes(
            "V4 payload attributes used for pre-Amsterdam timestamp".to_string(),
        ));
    }
    if attributes.withdrawals.is_none() {
        return Err(RpcErr::InvalidPayloadAttributes(
            "V4 payload attributes missing withdrawals".to_string(),
        ));
    }
    if attributes.parent_beacon_block_root.is_none() {
        return Err(RpcErr::InvalidPayloadAttributes(
            "V4 payload attributes missing parent_beacon_block_root".to_string(),
        ));
    }
    // execution-apis#796: target_gas_limit is required on V4 and enforced at
    // deserialization (see `parse_v4`), so no presence check is needed here.
    validate_timestamp_v4(attributes, head_block)
}

fn validate_timestamp_v4(
    attributes: &PayloadAttributesV4,
    head_block: &BlockHeader,
) -> Result<(), RpcErr> {
    if attributes.timestamp <= head_block.timestamp {
        return Err(RpcErr::InvalidPayloadAttributes(
            "invalid timestamp".to_string(),
        ));
    }
    Ok(())
}

async fn build_payload_v4(
    attributes: &PayloadAttributesV4,
    context: RpcApiContext,
    fork_choice_state: &ForkChoiceState,
) -> Result<u64, RpcErr> {
    // execution-apis#796: use the CL-supplied target gas limit (required on V4).
    let gas_ceil = attributes.target_gas_limit;
    let args = BuildPayloadArgs {
        parent: fork_choice_state.head_block_hash,
        timestamp: attributes.timestamp,
        fee_recipient: attributes.suggested_fee_recipient,
        random: attributes.prev_randao,
        withdrawals: attributes.withdrawals.clone(),
        beacon_root: attributes.parent_beacon_block_root,
        slot_number: Some(attributes.slot_number),
        version: 4,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil,
    };
    let payload_id = args
        .id()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;

    info!(
        id = payload_id,
        slot = attributes.slot_number,
        gas_ceil,
        "Fork choice updated V4 includes payload attributes. Creating a new payload"
    );
    let payload = match create_payload(&args, &context.storage, context.node_data.extra_data) {
        Ok(payload) => payload,
        Err(ChainError::EvmError(error)) => return Err(error.into()),
        Err(error) => return Err(RpcErr::Internal(error.to_string())),
    };
    context
        .blockchain
        .initiate_payload_build(payload, payload_id)
        .await;
    Ok(payload_id)
}

#[cfg(test)]
mod tests {
    use super::{validate_attributes_v2, validate_attributes_v2_pre_shanghai};
    use crate::types::fork_choice::PayloadAttributesV3;
    use ethrex_common::types::{BlockHeader, Withdrawal};

    // NOTE: the custodyColumns / parse_v4 / apply_custody_update (eth/72,
    // EIP-8070) tests were moved to test/tests/rpc/fork_choice_tests.rs.

    #[test]
    fn forkchoice_updated_v2_returns_invalid_payload_attributes_when_withdrawals_missing() {
        let attributes = PayloadAttributesV3 {
            timestamp: 2,
            withdrawals: None,
            ..Default::default()
        };
        let head_block = BlockHeader {
            timestamp: 1,
            ..Default::default()
        };

        let err = validate_attributes_v2(&attributes, &head_block).unwrap_err();

        assert!(matches!(
            err,
            crate::utils::RpcErr::InvalidPayloadAttributes(_)
        ));
    }

    #[test]
    fn forkchoice_updated_v2_returns_invalid_payload_attributes_pre_shanghai_with_withdrawals() {
        let attributes = PayloadAttributesV3 {
            timestamp: 2,
            withdrawals: Some(Vec::<Withdrawal>::new()),
            ..Default::default()
        };
        let head_block = BlockHeader {
            timestamp: 1,
            ..Default::default()
        };

        let err = validate_attributes_v2_pre_shanghai(&attributes, &head_block).unwrap_err();

        assert!(matches!(
            err,
            crate::utils::RpcErr::InvalidPayloadAttributes(_)
        ));
    }
}
