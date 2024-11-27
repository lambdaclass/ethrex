use ethrex_blockchain::add_block;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::payload::build_payload;
use ethrex_core::types::Fork;
use ethrex_core::U256;
use serde_json::Value;
use tracing::{error, info, warn};

use crate::types::payload::{ExecutionPayload, ExecutionPayloadResponse, PayloadStatus};
use crate::utils::RpcRequest;
use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct NewPayloadV2Request {
    pub payload: ExecutionPayload,
}

pub struct GetPayloadV2Request {
    pub payload_id: u64,
}

impl From<NewPayloadV2Request> for RpcRequest {
    fn from(val: NewPayloadV2Request) -> Self {
        RpcRequest {
            method: "engine_newPayloadV2".to_string(),
            params: Some(vec![serde_json::json!(val.payload)]),
            ..Default::default()
        }
    }
}

impl RpcHandler for NewPayloadV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        Ok(NewPayloadV2Request {
            payload: serde_json::from_value(params[0].clone())
                .map_err(|_| RpcErr::WrongParam("payload".to_string()))?,
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;

        let block_hash = self.payload.block_hash;
        info!("Received new payload with block hash: {block_hash:#x}");

        let block = match self.payload.clone().into_block(None) {
            Ok(block) => block,
            Err(error) => {
                let result = PayloadStatus::invalid_with_err(&error.to_string());
                return serde_json::to_value(result)
                    .map_err(|error| RpcErr::Internal(error.to_string()));
            }
        };

        // Payload Validation

        // Check timestamp is post Shanghai fork
        let chain_config = storage.get_chain_config()?;
        let current_fork = chain_config.get_fork(block.header.timestamp);
        if current_fork < Fork::Shanghai {
            return Err(RpcErr::UnsuportedFork(format!("{current_fork:?}")));
        }

        // Check that block_hash is valid
        let actual_block_hash = block.hash();
        if block_hash != actual_block_hash {
            let result = PayloadStatus::invalid_with_err("Invalid block hash");
            return serde_json::to_value(result)
                .map_err(|error| RpcErr::Internal(error.to_string()));
        }

        info!("Block hash {block_hash} is valid");
        // Return the valid message directly if we have it.
        if storage.get_block_header_by_hash(block_hash)?.is_some() {
            let result = PayloadStatus::valid_with_hash(block_hash);
            return serde_json::to_value(result)
                .map_err(|error| RpcErr::Internal(error.to_string()));
        }

        // Execute and store the block
        info!("Executing payload with block hash: {block_hash:#x}");
        let payload_status = match add_block(&block, storage) {
            Err(ChainError::ParentNotFound) => Ok(PayloadStatus::syncing()),
            // Under the current implementation this is not possible: we always calculate the state
            // transition of any new payload as long as the parent is present. If we received the
            // parent payload but it was stashed, then new payload would stash this one too, with a
            // ParentNotFoundError.
            Err(ChainError::ParentStateNotFound) => {
                let e = "Failed to obtain parent state";
                error!("{e} for block {block_hash}");
                Err(RpcErr::Internal(e.to_string()))
            }
            Err(ChainError::InvalidBlock(error)) => {
                warn!("Error adding block: {error}");
                // TODO(#982): this is only valid for the cases where the parent was found, but fully invalid ones may also happen.
                Ok(PayloadStatus::invalid_with(
                    block.header.parent_hash,
                    error.to_string(),
                ))
            }
            Err(ChainError::EvmError(error)) => {
                warn!("Error executing block: {error}");
                Ok(PayloadStatus::invalid_with(
                    block.header.parent_hash,
                    error.to_string(),
                ))
            }
            Err(ChainError::StoreError(error)) => {
                warn!("Error storing block: {error}");
                Err(RpcErr::Internal(error.to_string()))
            }
            Ok(()) => {
                info!("Block with hash {block_hash} executed and added to storage succesfully");
                Ok(PayloadStatus::valid_with_hash(block_hash))
            }
        }?;

        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl From<GetPayloadV2Request> for RpcRequest {
    fn from(val: GetPayloadV2Request) -> Self {
        RpcRequest {
            method: "engine_getPayloadV2".to_string(),
            params: Some(vec![serde_json::json!(U256::from(val.payload_id))]),
            ..Default::default()
        }
    }
}

impl RpcHandler for GetPayloadV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        let Ok(hex_str) = serde_json::from_value::<String>(params[0].clone()) else {
            return Err(RpcErr::BadParams(
                "Expected param to be a string".to_owned(),
            ));
        };
        // Check that the hex string is 0x prefixed
        let Some(hex_str) = hex_str.strip_prefix("0x") else {
            return Err(RpcErr::BadHexFormat(0));
        };
        // Parse hex string
        let Ok(payload_id) = u64::from_str_radix(hex_str, 16) else {
            return Err(RpcErr::BadHexFormat(0));
        };
        Ok(GetPayloadV2Request { payload_id })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!("Requested payload with id: {:#018x}", self.payload_id);
        let Some(mut payload) = context.storage.get_payload(self.payload_id)? else {
            return Err(RpcErr::UnknownPayload(format!(
                "Payload with id {:#018x} not found",
                self.payload_id
            )));
        };
        let (_blobs_bundle, block_value) = build_payload(&mut payload, &context.storage)
            .map_err(|err| RpcErr::Internal(err.to_string()))?;
        serde_json::to_value(ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(payload),
            block_value,
            blobs_bundle: None,
            should_override_builder: None,
        })
        .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
