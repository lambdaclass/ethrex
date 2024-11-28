use ethrex_blockchain::{
    error::{ChainError, InvalidForkChoice},
    fork_choice::apply_fork_choice,
    latest_canonical_block_hash,
    payload::{create_payload, BuildPayloadArgs},
};
use ethrex_core::types::{BlockHeader, ChainConfig};
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    types::{
        fork_choice::{ForkChoiceResponse, ForkChoiceState, PayloadAttributes},
        payload::PayloadStatus,
    },
    utils::RpcRequest,
    RpcApiContext, RpcErr, RpcHandler,
};

#[derive(Debug)]
pub struct ForkChoiceUpdated {
    pub fork_choice_state: ForkChoiceState,
    pub payload_attributes: Result<Option<PayloadAttributes>, String>,
}

impl ForkChoiceUpdated {
    // TODO(#853): Allow fork choice to be executed even if fork choice updated was not correctly parsed.
    fn parse(params: &Option<Vec<Value>>) -> Result<ForkChoiceUpdated, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        }
        Ok(ForkChoiceUpdated {
            fork_choice_state: serde_json::from_value(params[0].clone())?,
            payload_attributes: serde_json::from_value(params[1].clone())
                .map_err(|e| e.to_string()),
        })
    }

    fn try_from(fork_choice_updated: &dyn ForkChoiceUpdatedImpl) -> Result<RpcRequest, String> {
        let request = fork_choice_updated.request();
        match &request.payload_attributes {
            Ok(attrs) => Ok(RpcRequest {
                method: fork_choice_updated.method(),
                params: Some(vec![
                    serde_json::json!(request.fork_choice_state),
                    serde_json::json!(attrs),
                ]),
                ..Default::default()
            }),
            Err(err) => Err(err.to_string()),
        }
    }

    fn handle(
        fork_choice_updated: &dyn ForkChoiceUpdatedImpl,
        context: RpcApiContext,
    ) -> Result<Value, RpcErr> {
        let request = fork_choice_updated.request();
        let storage = &context.storage;
        info!(
            "New fork choice request with head: {}, safe: {}, finalized: {}.",
            request.fork_choice_state.head_block_hash,
            request.fork_choice_state.safe_block_hash,
            request.fork_choice_state.finalized_block_hash
        );
        let fork_choice_error_to_response = |error| {
            let response = match error {
                InvalidForkChoice::NewHeadAlreadyCanonical => ForkChoiceResponse::from(
                    PayloadStatus::valid_with_hash(latest_canonical_block_hash(storage).unwrap()),
                ),
                InvalidForkChoice::Syncing => ForkChoiceResponse::from(PayloadStatus::syncing()),
                reason => {
                    warn!("Invalid fork choice state. Reason: {:#?}", reason);
                    return Err(RpcErr::InvalidForkChoiceState(reason.to_string()));
                }
            };

            serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
        };

        let head_block = match apply_fork_choice(
            storage,
            request.fork_choice_state.head_block_hash,
            request.fork_choice_state.safe_block_hash,
            request.fork_choice_state.finalized_block_hash,
        ) {
            Ok(head) => head,
            Err(error) => return fork_choice_error_to_response(error),
        };

        // Build block from received payload. This step is skipped if applying the fork choice state failed
        let mut response = ForkChoiceResponse::from(PayloadStatus::valid_with_hash(
            request.fork_choice_state.head_block_hash,
        ));

        match &request.payload_attributes {
            // Payload may be invalid but we had to apply fork choice state nevertheless.
            Err(e) => return Err(RpcErr::InvalidPayloadAttributes(e.into())),
            Ok(None) => (),
            Ok(Some(attributes)) => {
                info!("Fork choice updated includes payload attributes. Creating a new payload.");
                let chain_config = storage.get_chain_config()?;
                fork_choice_updated.validate(attributes, chain_config, head_block)?;
                let args = BuildPayloadArgs {
                    parent: request.fork_choice_state.head_block_hash,
                    timestamp: attributes.timestamp,
                    fee_recipient: attributes.suggested_fee_recipient,
                    random: attributes.prev_randao,
                    withdrawals: attributes.withdrawals.clone(),
                    beacon_root: attributes.parent_beacon_block_root,
                    version: fork_choice_updated.version(),
                };
                let payload_id = args.id();
                response.set_id(payload_id);
                let payload = match create_payload(&args, storage) {
                    Ok(payload) => payload,
                    Err(ChainError::EvmError(error)) => return Err(error.into()),
                    // Parent block is guaranteed to be present at this point,
                    // so the only errors that may be returned are internal storage errors
                    Err(error) => return Err(RpcErr::Internal(error.to_string())),
                };
                storage.add_payload(payload_id, payload)?;
            }
        }

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

trait ForkChoiceUpdatedImpl {
    fn method(&self) -> String;
    fn request(&self) -> &ForkChoiceUpdated;
    fn version(&self) -> u8;
    fn validate(
        &self,
        attributes: &PayloadAttributes,
        chain_config: ChainConfig,
        head_block: BlockHeader,
    ) -> Result<(), RpcErr>;
}

#[derive(Debug)]
pub struct ForkChoiceUpdatedV2(ForkChoiceUpdated);

impl ForkChoiceUpdatedImpl for ForkChoiceUpdatedV2 {
    fn method(&self) -> String {
        "engine_forkchoiceUpdatedV2".to_string()
    }

    fn request(&self) -> &ForkChoiceUpdated {
        &self.0
    }

    fn version(&self) -> u8 {
        2
    }

    fn validate(
        &self,
        attributes: &PayloadAttributes,
        chain_config: ChainConfig,
        head_block: BlockHeader,
    ) -> Result<(), RpcErr> {
        if attributes.parent_beacon_block_root.is_some() {
            return Err(RpcErr::InvalidPayloadAttributes(
                "forkChoiceV2 with Beacon Root".to_string(),
            ));
        }
        if !chain_config.is_shanghai_activated(attributes.timestamp) {
            return Err(RpcErr::UnsuportedFork(
                "forkChoiceV2 used to build pre-Shanghai payload".to_string(),
            ));
        }
        if chain_config.is_cancun_activated(attributes.timestamp) {
            return Err(RpcErr::UnsuportedFork(
                "forkChoiceV2 used to build Cancun payload".to_string(),
            ));
        }
        if attributes.timestamp <= head_block.timestamp {
            return Err(RpcErr::InvalidPayloadAttributes(
                "invalid timestamp".to_string(),
            ));
        }
        Ok(())
    }
}

impl TryFrom<ForkChoiceUpdatedV2> for RpcRequest {
    type Error = String;

    fn try_from(val: ForkChoiceUpdatedV2) -> Result<Self, Self::Error> {
        ForkChoiceUpdated::try_from(&val)
    }
}

impl RpcHandler for ForkChoiceUpdatedV2 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self(ForkChoiceUpdated::parse(params)?))
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        ForkChoiceUpdated::handle(self, context)
    }
}

#[derive(Debug)]
pub struct ForkChoiceUpdatedV3(pub ForkChoiceUpdated);

impl ForkChoiceUpdatedImpl for ForkChoiceUpdatedV3 {
    fn method(&self) -> String {
        "engine_forkchoiceUpdatedV3".to_string()
    }

    fn request(&self) -> &ForkChoiceUpdated {
        &self.0
    }

    fn version(&self) -> u8 {
        3
    }

    fn validate(
        &self,
        attributes: &PayloadAttributes,
        chain_config: ChainConfig,
        head_block: BlockHeader,
    ) -> Result<(), RpcErr> {
        if attributes.parent_beacon_block_root.is_none() {
            return Err(RpcErr::InvalidPayloadAttributes(
                "Null Parent Beacon Root".to_string(),
            ));
        }
        if !chain_config.is_cancun_activated(attributes.timestamp) {
            return Err(RpcErr::UnsuportedFork(
                "forkChoiceV3 used to build pre-Cancun payload".to_string(),
            ));
        }
        if attributes.timestamp <= head_block.timestamp {
            return Err(RpcErr::InvalidPayloadAttributes(
                "invalid timestamp".to_string(),
            ));
        }
        Ok(())
    }
}

impl TryFrom<ForkChoiceUpdatedV3> for RpcRequest {
    type Error = String;

    fn try_from(val: ForkChoiceUpdatedV3) -> Result<Self, Self::Error> {
        ForkChoiceUpdated::try_from(&val)
    }
}

impl RpcHandler for ForkChoiceUpdatedV3 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self(ForkChoiceUpdated::parse(params)?))
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        ForkChoiceUpdated::handle(self, context)
    }
}
