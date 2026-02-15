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
                // Behave as a v1
                validate_attributes_v1(attributes, &head_block)?;
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
        return Err(RpcErr::WrongParam("withdrawals".to_string()));
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
        .initiate_payload_build(payload, payload_id)
        .await;
    Ok(payload_id)
}
