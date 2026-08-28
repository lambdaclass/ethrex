use ethrex_blockchain::{
    error::{ChainError, InvalidForkChoice},
    fork_choice::apply_fork_choice_with_deep_reorg,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::types::{BlockHeader, ELASTICITY_MULTIPLIER, Transaction};
use ethrex_p2p::sync::SyncMode;
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::{
    engine::inclusion_list::{block_satisfies_inclusion_list, decode_inclusion_list},
    rpc::{RpcApiContext, RpcHandler},
    subscription_manager::SubscriptionManagerProtocol,
    types::{
        fork_choice::{
            ForkChoiceResponse, ForkChoiceState, PayloadAttributesV3, PayloadAttributesV4,
            PayloadAttributesV5,
        },
        payload::{PayloadStatus, PayloadValidationStatus},
    },
    utils::RpcErr,
    utils::RpcRequest,
};

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
}

impl From<ForkChoiceUpdatedV4> for RpcRequest {
    fn from(val: ForkChoiceUpdatedV4) -> Self {
        RpcRequest {
            method: "engine_forkchoiceUpdatedV4".to_string(),
            params: Some(vec![
                serde_json::json!(val.fork_choice_state),
                serde_json::json!(val.payload_attributes),
            ]),
            ..Default::default()
        }
    }
}

impl RpcHandler for ForkChoiceUpdatedV4 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let (fork_choice_state, payload_attributes) = parse_v4(params)?;
        Ok(ForkChoiceUpdatedV4 {
            fork_choice_state,
            payload_attributes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let (head_block_opt, mut response) =
            handle_forkchoice(&self.fork_choice_state, context.clone(), 4).await?;
        if let (Some(head_block), Some(attributes)) = (head_block_opt, &self.payload_attributes) {
            let chain_config = context.storage.get_chain_config();
            validate_attributes_v4(attributes, &head_block, &chain_config)?;
            let payload_id = build_payload_v4(attributes, context, &self.fork_choice_state).await?;
            response.set_id(payload_id);
        }
        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

/// `engine_forkchoiceUpdatedV5` — the Bogotá fork choice call (EIP-7805).
///
/// Beyond V4 it does two things: it reports whether the head it is told to
/// adopt satisfied its inclusion list, reusing the list `engine_newPayloadV6`
/// retained for that block, and it accepts an inclusion list in the payload
/// attributes for the block it is asked to build.
#[derive(Debug)]
pub struct ForkChoiceUpdatedV5 {
    pub fork_choice_state: ForkChoiceState,
    pub payload_attributes: Option<PayloadAttributesV5>,
}

impl From<ForkChoiceUpdatedV5> for RpcRequest {
    fn from(val: ForkChoiceUpdatedV5) -> Self {
        RpcRequest {
            method: "engine_forkchoiceUpdatedV5".to_string(),
            params: Some(vec![
                serde_json::json!(val.fork_choice_state),
                serde_json::json!(val.payload_attributes),
            ]),
            ..Default::default()
        }
    }
}

impl RpcHandler for ForkChoiceUpdatedV5 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let (fork_choice_state, payload_attributes) = parse_v5(params)?;
        Ok(ForkChoiceUpdatedV5 {
            fork_choice_state,
            payload_attributes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let (head_block_opt, mut response) =
            handle_forkchoice(&self.fork_choice_state, context.clone(), 5).await?;

        // EIP-7805: report the head's inclusion-list verdict from the list
        // `engine_newPayloadV6` retained for it. Only a `VALID` head carries a
        // verdict — execution-apis `bogota.md` requires `inclusionListSatisfied`
        // to be `null` for every other status. A head whose list was never
        // retained (evicted, or delivered before this node started) reports
        // nothing rather than guessing `true`.
        if response.payload_status.status == PayloadValidationStatus::Valid {
            let head_hash = self.fork_choice_state.head_block_hash;
            let retained = match context.retained_inclusion_lists.lock() {
                Ok(lists) => lists.get(&head_hash).map(<[Transaction]>::to_vec),
                Err(e) => {
                    return Err(RpcErr::Internal(format!(
                        "retained inclusion list lock poisoned: {e}"
                    )));
                }
            };
            if let Some(inclusion_list) = retained {
                let satisfied =
                    block_satisfies_inclusion_list(&context, head_hash, &inclusion_list).await?;
                response.payload_status.inclusion_list_satisfied = Some(satisfied);
            }
        }

        if let (Some(head_block), Some(attributes)) = (head_block_opt, &self.payload_attributes) {
            let chain_config = context.storage.get_chain_config();
            validate_attributes_v5(attributes, &head_block, &chain_config)?;
            let payload_id = build_payload_v5(attributes, context, &self.fork_choice_state).await?;
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

    match apply_fork_choice_with_deep_reorg(
        &context.blockchain,
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
        inclusion_list_transactions: None,
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
        .initiate_payload_build(payload, payload_id, Vec::new())
        .await;
    Ok(payload_id)
}

fn parse_v4(
    params: &Option<Vec<Value>>,
) -> Result<(ForkChoiceState, Option<PayloadAttributesV4>), RpcErr> {
    let params = params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;

    if params.len() != 2 && params.len() != 1 {
        return Err(RpcErr::BadParams("Expected 2 or 1 params".to_owned()));
    }

    let forkchoice_state: ForkChoiceState = serde_json::from_value(params[0].clone())?;
    let mut payload_attributes: Option<PayloadAttributesV4> = None;
    if params.len() == 2 {
        // execution-apis#796: V4 attributes are validated strictly. A present but
        // malformed object (e.g. missing the required targetGasLimit) is rejected
        // rather than silently ignored; an absent/null object yields no attributes.
        payload_attributes = serde_json::from_value::<Option<PayloadAttributesV4>>(
            params[1].clone(),
        )
        .map_err(|error| {
            RpcErr::InvalidPayloadAttributes(format!("invalid V4 payload attributes: {error}"))
        })?;
    }
    Ok((forkchoice_state, payload_attributes))
}

fn validate_attributes_v4(
    attributes: &PayloadAttributesV4,
    head_block: &BlockHeader,
    chain_config: &ethrex_common::types::ChainConfig,
) -> Result<(), RpcErr> {
    // Bogotá payload attributes must come through V5, which is the only version
    // that carries an inclusion list. Building from V4 attributes on a
    // Bogotá-active timestamp would silently produce a block under no
    // inclusion-list obligation.
    if chain_config.is_hegota_activated(attributes.timestamp) {
        return Err(RpcErr::UnsupportedFork(
            "engine_forkchoiceUpdatedV4 cannot accept Bogotá payload attributes".to_string(),
        ));
    }
    // Similar validation to V3
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
        inclusion_list_transactions: None,
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
        .initiate_payload_build(payload, payload_id, Vec::new())
        .await;
    Ok(payload_id)
}

fn parse_v5(
    params: &Option<Vec<Value>>,
) -> Result<(ForkChoiceState, Option<PayloadAttributesV5>), RpcErr> {
    let params = params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;

    // Arity mirrors `parse_v4`. Both `amsterdam.md` and `bogota.md` define a
    // third `custodyColumns` parameter, which this client does not implement on
    // either version; accepting it only on V5 would make the two inconsistent
    // without making a custody-providing consensus layer work, since it would
    // still be rejected on V4. Fixing both belongs in its own change.
    if params.len() != 2 && params.len() != 1 {
        return Err(RpcErr::BadParams("Expected 2 or 1 params".to_owned()));
    }

    let forkchoice_state: ForkChoiceState = serde_json::from_value(params[0].clone())?;
    let mut payload_attributes: Option<PayloadAttributesV5> = None;
    if params.len() == 2 {
        // execution-apis#796: V5 attributes are validated strictly, mirroring
        // parse_v4. A present but malformed object (e.g. missing the required
        // targetGasLimit) is rejected rather than silently ignored; an
        // absent/null object yields no attributes.
        payload_attributes = serde_json::from_value::<Option<PayloadAttributesV5>>(
            params[1].clone(),
        )
        .map_err(|error| {
            RpcErr::InvalidPayloadAttributes(format!("invalid V5 payload attributes: {error}"))
        })?;
    }
    Ok((forkchoice_state, payload_attributes))
}

fn validate_attributes_v5(
    attributes: &PayloadAttributesV5,
    head_block: &BlockHeader,
    chain_config: &ethrex_common::types::ChainConfig,
) -> Result<(), RpcErr> {
    // V5 is the Bogotá-and-later FCU: a pre-Bogotá timestamp belongs to V4 or
    // earlier and is rejected with -38005, mirroring how V4 rejects pre-Amsterdam.
    if !chain_config.is_hegota_activated(attributes.timestamp) {
        return Err(RpcErr::UnsupportedFork(
            "V5 payload attributes used for pre-Bogotá timestamp".to_string(),
        ));
    }
    if attributes.withdrawals.is_none() {
        return Err(RpcErr::InvalidPayloadAttributes(
            "V5 payload attributes missing withdrawals".to_string(),
        ));
    }
    if attributes.parent_beacon_block_root.is_none() {
        return Err(RpcErr::InvalidPayloadAttributes(
            "V5 payload attributes missing parent_beacon_block_root".to_string(),
        ));
    }
    // execution-apis#796: target_gas_limit is required on V5 as on V4, and is
    // enforced at deserialization (see `parse_v5`).
    if attributes.timestamp <= head_block.timestamp {
        return Err(RpcErr::InvalidPayloadAttributes(
            "invalid timestamp".to_string(),
        ));
    }
    Ok(())
}

/// Decodes the inclusion list and starts the build with it, so the locally
/// built block honours the list during construction. `engine_newPayloadV6`
/// remains the authority on whether a *received* block satisfied one.
async fn build_payload_v5(
    attributes: &PayloadAttributesV5,
    context: RpcApiContext,
    fork_choice_state: &ForkChoiceState,
) -> Result<u64, RpcErr> {
    // An inclusion-list entry that does not decode is skipped, not fatal: the
    // list is untrusted input from another party's node, and EIP-7805 gives the
    // execution layer no way to reject one entry without rejecting the whole
    // forkchoice call. There is no size cap on this path either — the 8 KiB
    // `MAX_BYTES_PER_INCLUSION_LIST` bounds what an execution layer BUILDS in
    // `engine_getInclusionListV1`, not what it accepts.
    let decoded_il = decode_inclusion_list(
        &attributes.inclusion_list_transactions,
        "engine_forkchoiceUpdatedV5",
    );
    let il_count = decoded_il.len();

    // execution-apis#796: use the CL-supplied target gas limit (required on V5).
    let gas_ceil = attributes.target_gas_limit;

    let args = BuildPayloadArgs {
        parent: fork_choice_state.head_block_hash,
        timestamp: attributes.timestamp,
        fee_recipient: attributes.suggested_fee_recipient,
        random: attributes.prev_randao,
        withdrawals: attributes.withdrawals.clone(),
        beacon_root: attributes.parent_beacon_block_root,
        slot_number: Some(attributes.slot_number),
        version: 5,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil,
        inclusion_list_transactions: if decoded_il.is_empty() {
            None
        } else {
            Some(decoded_il.clone())
        },
    };
    let payload_id = args
        .id()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;

    info!(
        id = payload_id,
        slot = attributes.slot_number,
        il_count,
        gas_ceil,
        "Fork choice updated V5 includes Bogotá payload attributes. Creating a new payload"
    );
    let payload = match create_payload(&args, &context.storage, context.node_data.extra_data) {
        Ok(payload) => payload,
        Err(ChainError::EvmError(error)) => return Err(error.into()),
        Err(error) => return Err(RpcErr::Internal(error.to_string())),
    };
    context
        .blockchain
        .initiate_payload_build(payload, payload_id, decoded_il)
        .await;
    Ok(payload_id)
}

#[cfg(test)]
mod tests {
    use super::{
        BuildPayloadArgs, ELASTICITY_MULTIPLIER, Transaction, validate_attributes_v2,
        validate_attributes_v2_pre_shanghai, validate_attributes_v4, validate_attributes_v5,
    };
    use crate::types::fork_choice::{
        PayloadAttributesV3, PayloadAttributesV4, PayloadAttributesV5,
    };
    use ethrex_common::types::{BlockHeader, Withdrawal};

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

    fn bogota_config() -> ethrex_common::types::ChainConfig {
        ethrex_common::types::ChainConfig {
            chain_id: 1,
            amsterdam_time: Some(500),
            hegota_time: Some(1000),
            ..Default::default()
        }
    }

    fn head_at(timestamp: u64) -> BlockHeader {
        BlockHeader {
            timestamp,
            ..Default::default()
        }
    }

    /// Bogotá attributes must arrive on V5, the only version that carries an
    /// inclusion list. V4 answers -38005 rather than building a block that is
    /// under no inclusion-list obligation.
    #[test]
    fn forkchoice_updated_v4_rejects_bogota_timestamp_with_unsupported_fork() {
        let attributes = PayloadAttributesV4 {
            timestamp: 1500,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            target_gas_limit: 60_000_000,
            ..Default::default()
        };
        let err =
            validate_attributes_v4(&attributes, &head_at(1499), &bogota_config()).unwrap_err();
        assert!(
            matches!(err, crate::utils::RpcErr::UnsupportedFork(_)),
            "expected UnsupportedFork, got {err:?}"
        );
    }

    /// The V4 guard must not fire on a chain that never schedules Bogotá.
    #[test]
    fn forkchoice_updated_v4_accepts_amsterdam_timestamp_without_bogota() {
        let chain_config = ethrex_common::types::ChainConfig {
            hegota_time: None,
            ..bogota_config()
        };
        let attributes = PayloadAttributesV4 {
            timestamp: 1500,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            target_gas_limit: 60_000_000,
            ..Default::default()
        };
        validate_attributes_v4(&attributes, &head_at(1499), &chain_config)
            .expect("V4 must still serve an Amsterdam-only chain");
    }

    #[test]
    fn validate_v5_rejects_pre_bogota_timestamp_with_unsupported_fork() {
        // 800 is Amsterdam (past 500) but pre-Bogotá (before 1000).
        let attributes = PayloadAttributesV5 {
            timestamp: 800,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            target_gas_limit: 60_000_000,
            ..Default::default()
        };
        let err = validate_attributes_v5(&attributes, &head_at(799), &bogota_config()).unwrap_err();
        assert!(
            matches!(err, crate::utils::RpcErr::UnsupportedFork(_)),
            "expected UnsupportedFork, got {err:?}"
        );
    }

    #[test]
    fn validate_v5_accepts_bogota_timestamp_with_empty_inclusion_list() {
        let attributes = PayloadAttributesV5 {
            timestamp: 1500,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            inclusion_list_transactions: vec![],
            target_gas_limit: 50_000_000,
            ..Default::default()
        };
        validate_attributes_v5(&attributes, &head_at(1499), &bogota_config())
            .expect("V5 must accept a Bogotá timestamp with an empty inclusion list");
    }

    #[test]
    fn validate_v5_rejects_missing_withdrawals() {
        let attributes = PayloadAttributesV5 {
            timestamp: 1500,
            withdrawals: None,
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            target_gas_limit: 60_000_000,
            ..Default::default()
        };
        let err =
            validate_attributes_v5(&attributes, &head_at(1499), &bogota_config()).unwrap_err();
        assert!(matches!(
            err,
            crate::utils::RpcErr::InvalidPayloadAttributes(_)
        ));
    }

    /// Two slots that differ only in their inclusion list must not collide on
    /// the payload id, or the second build would be served the first's payload.
    #[test]
    fn inclusion_list_changes_the_payload_id() {
        let base = || BuildPayloadArgs {
            parent: Default::default(),
            timestamp: 1500,
            fee_recipient: Default::default(),
            random: Default::default(),
            withdrawals: Some(vec![]),
            beacon_root: Some(Default::default()),
            slot_number: Some(1),
            version: 5,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: 60_000_000,
            inclusion_list_transactions: None,
        };
        let tx = Transaction::LegacyTransaction(Default::default());
        let without = base().id().expect("payload id");
        let with = BuildPayloadArgs {
            inclusion_list_transactions: Some(vec![tx]),
            ..base()
        }
        .id()
        .expect("payload id");
        assert_ne!(without, with);
    }
}
