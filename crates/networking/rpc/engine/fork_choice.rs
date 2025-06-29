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
        fork_choice::{ForkChoiceResponse, ForkChoiceState, PayloadAttributesV3},
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
            let chain_config = context.storage.get_chain_config()?;
            if chain_config.is_cancun_activated(attributes.timestamp) {
                return Err(RpcErr::UnsuportedFork(
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
            let chain_config = context.storage.get_chain_config()?;
            if chain_config.is_cancun_activated(attributes.timestamp) {
                return Err(RpcErr::UnsuportedFork(
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
                    info!("Could not parse params {}", error);
                    None
                }
            };
    }
    if let Some(attr) = &payload_attributes {
        if !is_v3 && attr.parent_beacon_block_root.is_some() {
            return Err(RpcErr::InvalidPayloadAttributes(
                "Attribute parent_beacon_block_root is non-null".to_string(),
            ));
        }
    }
    Ok((forkchoice_state, payload_attributes))
}

async fn handle_forkchoice(
    fork_choice_state: &ForkChoiceState,
    context: RpcApiContext,
    version: usize,
) -> Result<(Option<BlockHeader>, ForkChoiceResponse), RpcErr> {
    debug!(
        "New fork choice request v{} with head: {:#x}, safe: {:#x}, finalized: {:#x}.",
        version,
        fork_choice_state.head_block_hash,
        fork_choice_state.safe_block_hash,
        fork_choice_state.finalized_block_hash
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
    {
        if let Some(latest_valid_hash) = context
            .storage
            .get_latest_valid_ancestor(head_block.parent_hash)
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
    }

    if context.syncer.sync_mode() == SyncMode::Snap {
        context
            .syncer
            .sync_to_head(fork_choice_state.head_block_hash);
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
                    for tx in &block.body.transactions {
                        context
                            .blockchain
                            .remove_transaction_from_pool(&tx.compute_hash())
                            .map_err(|err| RpcErr::Internal(err.to_string()))?;
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
                    context
                        .syncer
                        .sync_to_head(fork_choice_state.head_block_hash);
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
    let chain_config = context.storage.get_chain_config()?;
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
        return Err(RpcErr::UnsuportedFork(
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
    info!("Fork choice updated includes payload attributes. Creating a new payload.");
    let args = BuildPayloadArgs {
        parent: fork_choice_state.head_block_hash,
        timestamp: attributes.timestamp,
        fee_recipient: attributes.suggested_fee_recipient,
        random: attributes.prev_randao,
        withdrawals: attributes.withdrawals.clone(),
        beacon_root: attributes.parent_beacon_block_root,
        version,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
    };
    let payload_id = args
        .id()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    let payload = match create_payload(&args, &context.storage) {
        Ok(payload) => payload,
        Err(ChainError::EvmError(error)) => return Err(error.into()),
        // Parent block is guaranteed to be present at this point,
        // so the only errors that may be returned are internal storage errors
        Err(error) => return Err(RpcErr::Internal(error.to_string())),
    };
    context.storage.add_payload(payload_id, payload).await?;

    Ok(payload_id)
}
