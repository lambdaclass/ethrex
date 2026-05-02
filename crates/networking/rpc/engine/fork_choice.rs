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
    types::{
        fork_choice::{
            ForkChoiceResponse, ForkChoiceState, PayloadAttributesV3, PayloadAttributesV4,
            PayloadAttributesV5,
        },
        payload::PayloadStatus,
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
        finalized = %format!("v{:#x}", fork_choice_state.finalized_block_hash),
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

            Ok((
                Some(head),
                ForkChoiceResponse::from(PayloadStatus::valid_with_hash(
                    fork_choice_state.head_block_hash,
                )),
            ))
        }
        Err(forkchoice_error) => {
            let forkchoice_response = match forkchoice_error {
                InvalidForkChoice::NewHeadAlreadyCanonical => ForkChoiceResponse::from(
                    PayloadStatus::valid_with_hash(fork_choice_state.head_block_hash),
                ),
                InvalidForkChoice::Syncing => {
                    // Start sync
                    syncer.sync_to_head(fork_choice_state.head_block_hash);
                    ForkChoiceResponse::from(PayloadStatus::syncing())
                }
                // TODO(#5564): handle arbitrary reorgs
                InvalidForkChoice::StateNotReachable => {
                    // Ignore the FCU
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
        payload_attributes =
            match serde_json::from_value::<Option<PayloadAttributesV4>>(params[1].clone()) {
                Ok(attributes) => attributes,
                Err(error) => {
                    warn!("Could not parse payload attributes {}", error);
                    None
                }
            };
    }
    Ok((forkchoice_state, payload_attributes))
}

fn validate_attributes_v4(
    attributes: &PayloadAttributesV4,
    head_block: &BlockHeader,
    chain_config: &ethrex_common::types::ChainConfig,
) -> Result<(), RpcErr> {
    // Pre-Hegotá guard: V4 cannot accept Hegotá-timestamp payload attributes.
    // Runs unconditionally (not feature-gated) so a non-FOCIL build still rejects
    // when the chain config has hegota_time set. The FCU state update is not
    // rolled back; only the payload-build request is rejected.
    if chain_config.is_hegota_activated(attributes.timestamp) {
        return Err(RpcErr::UnsupportedFork(
            "engine_forkchoiceUpdatedV4 cannot accept Hegotá payload attributes".to_string(),
        ));
    }
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
        gas_ceil: context.gas_ceil,
        inclusion_list_transactions: None,
    };
    let payload_id = args
        .id()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;

    info!(
        id = payload_id,
        slot = attributes.slot_number,
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

    if params.len() != 2 && params.len() != 1 {
        return Err(RpcErr::BadParams("Expected 2 or 1 params".to_owned()));
    }

    let forkchoice_state: ForkChoiceState = serde_json::from_value(params[0].clone())?;
    let mut payload_attributes: Option<PayloadAttributesV5> = None;
    if params.len() == 2 {
        payload_attributes =
            match serde_json::from_value::<Option<PayloadAttributesV5>>(params[1].clone()) {
                Ok(attributes) => attributes,
                Err(error) => {
                    warn!("Could not parse V5 payload attributes {}", error);
                    None
                }
            };
    }
    Ok((forkchoice_state, payload_attributes))
}

fn validate_attributes_v5(
    attributes: &PayloadAttributesV5,
    head_block: &BlockHeader,
    chain_config: &ethrex_common::types::ChainConfig,
) -> Result<(), RpcErr> {
    // V5 is the Hegotá-and-later FCU. Reject any pre-Hegotá timestamp with
    // -38005, mirroring the spec.
    if !chain_config.is_hegota_activated(attributes.timestamp) {
        return Err(RpcErr::UnsupportedFork(
            "V5 payload attributes used for pre-Hegotá timestamp".to_string(),
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
    if attributes.timestamp <= head_block.timestamp {
        return Err(RpcErr::InvalidPayloadAttributes(
            "invalid timestamp".to_string(),
        ));
    }
    Ok(())
}

/// V5 payload-build hook. Decodes the IL transactions for log/observability
/// but does NOT yet thread them into `BuildPayloadArgs` — that wiring lands
/// in Phase 5.1 (`BuildPayloadArgs::inclusion_list_transactions`). For now,
/// the locally-built block does not honor the IL during construction; the
/// remote-validation path in `engine_newPayloadV6` is the authoritative
/// satisfaction check.
async fn build_payload_v5(
    attributes: &PayloadAttributesV5,
    context: RpcApiContext,
    fork_choice_state: &ForkChoiceState,
) -> Result<u64, RpcErr> {
    use ethrex_common::types::Transaction;

    // Defensive: log if we receive ILs we can't decode. The CL is responsible
    // for forwarding well-formed RLP; a bad encoding is a CL bug, not a
    // proposer-injection condition we should accept.
    let il_count = attributes.inclusion_list_transactions.len();
    let mut decoded_il: Vec<Transaction> = Vec::with_capacity(il_count);
    for (i, raw) in attributes.inclusion_list_transactions.iter().enumerate() {
        match Transaction::decode_canonical(raw.as_ref()) {
            Ok(tx) => decoded_il.push(tx),
            Err(e) => {
                return Err(RpcErr::InvalidPayloadAttributes(format!(
                    "inclusion_list_transactions[{i}]: RLP decode failed: {e}"
                )));
            }
        }
    }

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
        gas_ceil: context.gas_ceil,
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
        "Fork choice updated V5 includes Hegotá payload attributes. Creating a new payload"
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
    use super::{validate_attributes_v2, validate_attributes_v2_pre_shanghai};
    use crate::types::fork_choice::PayloadAttributesV3;
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

    #[test]
    fn forkchoice_updated_v4_rejects_hegota_timestamp_with_unsupported_fork() {
        use super::validate_attributes_v4;
        use crate::types::fork_choice::PayloadAttributesV4;
        use ethereum_types::Address;
        use ethrex_common::types::ChainConfig;

        let chain_config = ChainConfig {
            chain_id: 1,
            deposit_contract_address: Address::default(),
            amsterdam_time: Some(500),
            hegota_time: Some(1000),
            ..Default::default()
        };

        let attributes = PayloadAttributesV4 {
            timestamp: 1500,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            ..Default::default()
        };
        let head_block = BlockHeader {
            timestamp: 1499,
            ..Default::default()
        };

        let err = validate_attributes_v4(&attributes, &head_block, &chain_config).unwrap_err();
        assert!(
            matches!(err, crate::utils::RpcErr::UnsupportedFork(_)),
            "expected UnsupportedFork, got {err:?}"
        );
    }

    #[test]
    fn forkchoice_updated_v4_accepts_amsterdam_timestamp_when_hegota_unset() {
        use super::validate_attributes_v4;
        use crate::types::fork_choice::PayloadAttributesV4;
        use ethereum_types::Address;
        use ethrex_common::types::ChainConfig;

        let chain_config = ChainConfig {
            chain_id: 1,
            deposit_contract_address: Address::default(),
            amsterdam_time: Some(500),
            hegota_time: None,
            ..Default::default()
        };

        let attributes = PayloadAttributesV4 {
            timestamp: 1500,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            ..Default::default()
        };
        let head_block = BlockHeader {
            timestamp: 1499,
            ..Default::default()
        };

        validate_attributes_v4(&attributes, &head_block, &chain_config)
            .expect("validate_attributes_v4 should accept Amsterdam-only chain");
    }

    #[test]
    fn validate_v5_rejects_pre_hegota_timestamp_with_unsupported_fork() {
        use super::validate_attributes_v5;
        use crate::types::fork_choice::PayloadAttributesV5;
        use ethereum_types::Address;
        use ethrex_common::types::ChainConfig;

        let chain_config = ChainConfig {
            chain_id: 1,
            deposit_contract_address: Address::default(),
            amsterdam_time: Some(500),
            hegota_time: Some(1000),
            ..Default::default()
        };

        // timestamp 800 is Amsterdam (post-500) but pre-Hegotá (pre-1000).
        let attributes = PayloadAttributesV5 {
            timestamp: 800,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            inclusion_list_transactions: vec![],
            ..Default::default()
        };
        let head_block = BlockHeader {
            timestamp: 799,
            ..Default::default()
        };
        let err = validate_attributes_v5(&attributes, &head_block, &chain_config).unwrap_err();
        assert!(
            matches!(err, crate::utils::RpcErr::UnsupportedFork(_)),
            "expected UnsupportedFork, got {err:?}"
        );
    }

    #[test]
    fn validate_v5_accepts_hegota_timestamp_with_empty_il() {
        use super::validate_attributes_v5;
        use crate::types::fork_choice::PayloadAttributesV5;
        use ethereum_types::Address;
        use ethrex_common::types::ChainConfig;

        let chain_config = ChainConfig {
            chain_id: 1,
            deposit_contract_address: Address::default(),
            amsterdam_time: Some(500),
            hegota_time: Some(1000),
            ..Default::default()
        };

        let attributes = PayloadAttributesV5 {
            timestamp: 1500,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            inclusion_list_transactions: vec![],
            ..Default::default()
        };
        let head_block = BlockHeader {
            timestamp: 1499,
            ..Default::default()
        };
        validate_attributes_v5(&attributes, &head_block, &chain_config)
            .expect("V5 must accept Hegotá-active timestamp with empty IL");
    }

    #[test]
    fn validate_v5_rejects_missing_withdrawals() {
        use super::validate_attributes_v5;
        use crate::types::fork_choice::PayloadAttributesV5;
        use ethereum_types::Address;
        use ethrex_common::types::ChainConfig;

        let chain_config = ChainConfig {
            chain_id: 1,
            deposit_contract_address: Address::default(),
            hegota_time: Some(1000),
            ..Default::default()
        };

        let attributes = PayloadAttributesV5 {
            timestamp: 1500,
            withdrawals: None,
            parent_beacon_block_root: Some(Default::default()),
            slot_number: 1,
            inclusion_list_transactions: vec![],
            ..Default::default()
        };
        let head_block = BlockHeader {
            timestamp: 1499,
            ..Default::default()
        };
        let err = validate_attributes_v5(&attributes, &head_block, &chain_config).unwrap_err();
        assert!(matches!(
            err,
            crate::utils::RpcErr::InvalidPayloadAttributes(_)
        ));
    }
}
