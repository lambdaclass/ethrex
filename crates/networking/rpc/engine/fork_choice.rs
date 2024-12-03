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
pub enum ForkChoiceUpdatedVersion {
    V1,
    V2,
    V3,
}

#[derive(Debug)]
pub struct ForkChoiceUpdated {
    pub fork_choice_state: ForkChoiceState,
    pub payload_attributes: Result<Option<PayloadAttributes>, String>,
    pub version: ForkChoiceUpdatedVersion,
}

impl ForkChoiceUpdated {
    fn method(&self) -> String {
        match self.version {
            ForkChoiceUpdatedVersion::V1 => "engine_forkchoiceUpdatedV1".to_string(),
            ForkChoiceUpdatedVersion::V2 => "engine_forkchoiceUpdatedV2".to_string(),
            ForkChoiceUpdatedVersion::V3 => "engine_forkchoiceUpdatedV3".to_string(),
        }
    }

    fn version(&self) -> u8 {
        match self.version {
            ForkChoiceUpdatedVersion::V1 => 1u8,
            ForkChoiceUpdatedVersion::V2 => 2u8,
            ForkChoiceUpdatedVersion::V3 => 3u8,
        }
    }

    fn validate(
        &self,
        attributes: &PayloadAttributes,
        chain_config: ChainConfig,
        head_block: BlockHeader,
    ) -> Result<(), RpcErr> {
        match self.version {
            ForkChoiceUpdatedVersion::V1 => {
                // TODO
                Ok(())
            }
            ForkChoiceUpdatedVersion::V2 => {
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
            ForkChoiceUpdatedVersion::V3 => {
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
    }

    // TODO(#853): Allow fork choice to be executed even if fork choice updated was not correctly parsed.
    fn parse(
        params: &Option<Vec<Value>>,
        version: ForkChoiceUpdatedVersion,
    ) -> Result<ForkChoiceUpdated, RpcErr> {
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
            version,
        })
    }

    fn try_from(request: ForkChoiceUpdated) -> Result<RpcRequest, String> {
        match &request.payload_attributes {
            Ok(attrs) => Ok(RpcRequest {
                method: request.method(),
                params: Some(vec![
                    serde_json::json!(request.fork_choice_state),
                    serde_json::json!(attrs),
                ]),
                ..Default::default()
            }),
            Err(err) => Err(err.to_string()),
        }
    }

    fn handle(request: &ForkChoiceUpdated, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        info!(
            "New fork choice request with head: {}, safe: {}, finalized: {}.",
            request.fork_choice_state.head_block_hash,
            request.fork_choice_state.safe_block_hash,
            request.fork_choice_state.finalized_block_hash
        );

        let head_block = match apply_fork_choice(
            storage,
            request.fork_choice_state.head_block_hash,
            request.fork_choice_state.safe_block_hash,
            request.fork_choice_state.finalized_block_hash,
        ) {
            Ok(head) => head,
            Err(error) => {
                let fork_choice_response = match error {
                    InvalidForkChoice::NewHeadAlreadyCanonical => {
                        ForkChoiceResponse::from(PayloadStatus::valid_with_hash(
                            latest_canonical_block_hash(&context.storage).unwrap(),
                        ))
                    }
                    InvalidForkChoice::Syncing => {
                        // Start sync
                        let current_number = context.storage.get_latest_block_number()?.unwrap();
                        let Some(current_head) =
                            context.storage.get_canonical_block_hash(current_number)?
                        else {
                            return Err(RpcErr::Internal(
                                "Missing latest canonical block".to_owned(),
                            ));
                        };
                        let sync_head = request.fork_choice_state.head_block_hash;
                        tokio::spawn(async move {
                            // If we can't get hold of the syncer, then it means that there is an active sync in process
                            if let Ok(mut syncer) = context.syncer.try_lock() {
                                syncer
                                    .start_sync(current_head, sync_head, context.storage.clone())
                                    .await
                            }
                        });
                        ForkChoiceResponse::from(PayloadStatus::syncing())
                    }
                    reason => {
                        warn!("Invalid fork choice state. Reason: {:#?}", reason);
                        return Err(RpcErr::InvalidForkChoiceState(reason.to_string()));
                    }
                };
                return serde_json::to_value(fork_choice_response)
                    .map_err(|error| RpcErr::Internal(error.to_string()));
            }
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
                request.validate(attributes, chain_config, head_block)?;
                let args = BuildPayloadArgs {
                    parent: request.fork_choice_state.head_block_hash,
                    timestamp: attributes.timestamp,
                    fee_recipient: attributes.suggested_fee_recipient,
                    random: attributes.prev_randao,
                    withdrawals: attributes.withdrawals.clone(),
                    beacon_root: attributes.parent_beacon_block_root,
                    version: request.version(),
                };
                let payload_id = args.id();
                response.set_id(payload_id);
                let payload = match create_payload(&args, &context.storage) {
                    Ok(payload) => payload,
                    Err(ChainError::EvmError(error)) => return Err(error.into()),
                    // Parent block is guaranteed to be present at this point,
                    // so the only errors that may be returned are internal storage errors
                    Err(error) => return Err(RpcErr::Internal(error.to_string())),
                };
                context.storage.add_payload(payload_id, payload)?;
            }
        }

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[derive(Debug)]
pub struct ForkChoiceUpdatedV2(ForkChoiceUpdated);

impl TryFrom<ForkChoiceUpdatedV2> for RpcRequest {
    type Error = String;

    fn try_from(val: ForkChoiceUpdatedV2) -> Result<Self, Self::Error> {
        ForkChoiceUpdated::try_from(val.0)
    }
}

impl RpcHandler for ForkChoiceUpdatedV2 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self(ForkChoiceUpdated::parse(
            params,
            ForkChoiceUpdatedVersion::V2,
        )?))
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        ForkChoiceUpdated::handle(&self.0, context)
    }
}

#[derive(Debug)]
pub struct ForkChoiceUpdatedV3(pub ForkChoiceUpdated);

impl TryFrom<ForkChoiceUpdatedV3> for RpcRequest {
    type Error = String;

    fn try_from(val: ForkChoiceUpdatedV3) -> Result<Self, Self::Error> {
        ForkChoiceUpdated::try_from(val.0)
    }
}

impl RpcHandler for ForkChoiceUpdatedV3 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self(ForkChoiceUpdated::parse(
            params,
            ForkChoiceUpdatedVersion::V3,
        )?))
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        ForkChoiceUpdated::handle(&self.0, context)
    }
}
