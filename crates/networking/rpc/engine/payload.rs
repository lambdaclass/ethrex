use ethrex_blockchain::add_block;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::payload::build_payload;
use ethrex_core::types::{BlobsBundle, Block, Fork};
use ethrex_core::{H256, U256};
use serde_json::Value;
use tracing::{error, info, warn};

use crate::types::payload::{ExecutionPayload, ExecutionPayloadResponse, PayloadStatus};
use crate::utils::RpcRequest;
use crate::{RpcApiContext, RpcErr, RpcHandler};

#[derive(Debug)]
pub enum NewPayloadRequestVersion {
    V1,
    V2,
    V3 {
        expected_blob_versioned_hashes: Vec<H256>,
        parent_beacon_block_root: H256,
    },
}

pub struct NewPayloadRequest {
    pub payload: ExecutionPayload,
    pub version: NewPayloadRequestVersion,
}

impl NewPayloadRequest {
    fn from(&self) -> RpcRequest {
        match &self.version {
            NewPayloadRequestVersion::V1 => todo!(),
            NewPayloadRequestVersion::V2 => RpcRequest {
                method: "engine_newPayloadV2".to_string(),
                params: Some(vec![serde_json::json!(self.payload)]),
                ..Default::default()
            },
            NewPayloadRequestVersion::V3 {
                expected_blob_versioned_hashes,
                parent_beacon_block_root,
            } => RpcRequest {
                method: "engine_newPayloadV3".to_string(),
                params: Some(vec![
                    serde_json::json!(self.payload),
                    serde_json::json!(expected_blob_versioned_hashes),
                    serde_json::json!(parent_beacon_block_root),
                ]),
                ..Default::default()
            },
        }
    }

    fn parent_beacon_block_root(&self) -> Option<H256> {
        match self.version {
            NewPayloadRequestVersion::V1 => None,
            NewPayloadRequestVersion::V2 => None,
            NewPayloadRequestVersion::V3 {
                parent_beacon_block_root,
                ..
            } => Some(parent_beacon_block_root),
        }
    }

    fn validate_execution_payload(&self) -> Result<(), RpcErr> {
        match self.version {
            NewPayloadRequestVersion::V1 => Ok(()),
            NewPayloadRequestVersion::V2 => Ok(()),
            NewPayloadRequestVersion::V3 { .. } => {
                if self.payload.excess_blob_gas.is_none() {
                    return Err(RpcErr::WrongParam("excess_blob_gas".to_string()));
                }
                if self.payload.blob_gas_used.is_none() {
                    return Err(RpcErr::WrongParam("blob_gas_used".to_string()));
                }
                Ok(())
            }
        }
    }

    fn valid_fork(&self) -> Fork {
        match self.version {
            NewPayloadRequestVersion::V1 => Fork::Paris,
            NewPayloadRequestVersion::V2 => Fork::Shanghai,
            NewPayloadRequestVersion::V3 { .. } => Fork::Cancun,
        }
    }

    fn are_blob_versioned_hashes_invalid(&self, block: &Block) -> bool {
        match &self.version {
            NewPayloadRequestVersion::V1 => false,
            NewPayloadRequestVersion::V2 => false,
            NewPayloadRequestVersion::V3 {
                expected_blob_versioned_hashes,
                ..
            } => {
                // Concatenate blob versioned hashes lists (tx.blob_versioned_hashes) of each blob transaction included in the payload, respecting the order of inclusion
                // and check that the resulting array matches expected_blob_versioned_hashes
                let blob_versioned_hashes: Vec<H256> = block
                    .body
                    .transactions
                    .iter()
                    .flat_map(|tx| tx.blob_versioned_hashes())
                    .collect();
                *expected_blob_versioned_hashes != blob_versioned_hashes
            }
        }
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;

        let block_hash = self.payload.block_hash;
        info!("Received new payload with block hash: {block_hash:#x}");

        let block = match self
            .payload
            .clone()
            .into_block(self.parent_beacon_block_root())
        {
            Ok(block) => block,
            Err(error) => {
                let result = PayloadStatus::invalid_with_err(&error.to_string());
                return serde_json::to_value(result)
                    .map_err(|error| RpcErr::Internal(error.to_string()));
            }
        };

        // Payload Validation
        self.validate_execution_payload()?;

        // Check timestamp is post valid fork
        let chain_config = storage.get_chain_config()?;
        let current_fork = chain_config.get_fork(block.header.timestamp);
        if current_fork < self.valid_fork() {
            return Err(RpcErr::UnsuportedFork(format!("{current_fork:?}")));
        }

        // Check that block_hash is valid
        let actual_block_hash = block.hash();
        if block_hash != actual_block_hash {
            let result = PayloadStatus::invalid_with_err(&format!(
                "Invalid block hash. Expected {actual_block_hash:#x}, got {block_hash:#x}"
            ));
            return serde_json::to_value(result)
                .map_err(|error| RpcErr::Internal(error.to_string()));
        }

        info!("Block hash {block_hash} is valid");
        if self.are_blob_versioned_hashes_invalid(&block) {
            let result = PayloadStatus::invalid_with_err("Invalid blob_versioned_hashes");
            return serde_json::to_value(result)
                .map_err(|error| RpcErr::Internal(error.to_string()));
        }

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

pub struct NewPayloadV3Request {
    pub new_payload_request: NewPayloadRequest,
}

impl From<NewPayloadV3Request> for RpcRequest {
    fn from(val: NewPayloadV3Request) -> Self {
        val.new_payload_request.from()
    }
}

impl RpcHandler for NewPayloadV3Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 3 {
            return Err(RpcErr::BadParams("Expected 3 params".to_owned()));
        }
        Ok(NewPayloadV3Request {
            new_payload_request: NewPayloadRequest {
                payload: serde_json::from_value(params[0].clone())
                    .map_err(|_| RpcErr::WrongParam("payload".to_string()))?,
                version: NewPayloadRequestVersion::V3 {
                    expected_blob_versioned_hashes: serde_json::from_value(params[1].clone())
                        .map_err(|_| {
                            RpcErr::WrongParam("expected_blob_versioned_hashes".to_string())
                        })?,
                    parent_beacon_block_root: serde_json::from_value(params[2].clone())
                        .map_err(|_| RpcErr::WrongParam("parent_beacon_block_root".to_string()))?,
                },
            },
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        self.new_payload_request.handle(context)
    }
}

pub struct NewPayloadV2Request {
    pub new_payload_request: NewPayloadRequest,
}

impl From<NewPayloadV2Request> for RpcRequest {
    fn from(val: NewPayloadV2Request) -> Self {
        val.new_payload_request.from()
    }
}

impl RpcHandler for NewPayloadV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 params".to_owned()));
        }
        Ok(NewPayloadV2Request {
            new_payload_request: NewPayloadRequest {
                payload: serde_json::from_value(params[0].clone())
                    .map_err(|_| RpcErr::WrongParam("payload".to_string()))?,
                version: NewPayloadRequestVersion::V2,
            },
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        self.new_payload_request.handle(context)
    }
}

#[derive(Clone)]
pub enum GetPayloadRequestVersion {
    V1 = 1,
    V2 = 2,
    V3 = 3,
}

pub struct GetPayloadRequest {
    pub payload_id: u64,
    pub version: GetPayloadRequestVersion,
}

impl GetPayloadRequest {
    fn method(&self) -> String {
        format!("engine_getPayloadV{}", self.version.clone() as usize)
    }

    fn valid_fork(&self) -> Fork {
        match self.version {
            GetPayloadRequestVersion::V1 => Fork::Paris,
            GetPayloadRequestVersion::V2 => Fork::Shanghai,
            GetPayloadRequestVersion::V3 => Fork::Cancun,
        }
    }

    fn build_response(
        &self,
        execution_payload: ExecutionPayload,
        payload_blobs_bundle: BlobsBundle,
        block_value: U256,
    ) -> ExecutionPayloadResponse {
        let (blobs_bundle, should_override_builder) =
            if let GetPayloadRequestVersion::V3 = self.version {
                (Some(payload_blobs_bundle), Some(false))
            } else {
                (None, None)
            };
        ExecutionPayloadResponse {
            execution_payload,
            block_value,
            blobs_bundle,
            should_override_builder,
        }
    }

    fn parse(
        params: &Option<Vec<Value>>,
        version: GetPayloadRequestVersion,
    ) -> Result<GetPayloadRequest, RpcErr> {
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
        Ok(GetPayloadRequest {
            payload_id,
            version,
        })
    }

    fn from(&self) -> RpcRequest {
        RpcRequest {
            method: self.method(),
            params: Some(vec![serde_json::json!(U256::from(self.payload_id))]),
            ..Default::default()
        }
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!("Requested payload with id: {:#018x}", self.payload_id);
        let Some(mut payload) = context.storage.get_payload(self.payload_id)? else {
            return Err(RpcErr::UnknownPayload(format!(
                "Payload with id {:#018x} not found",
                self.payload_id
            )));
        };
        // Check timestamp matches valid fork
        let chain_config = &context.storage.get_chain_config()?;
        let current_fork = chain_config.get_fork(payload.header.timestamp);
        info!("Current Fork: {:?}", current_fork);
        if current_fork != self.valid_fork() {
            return Err(RpcErr::UnsuportedFork(format!("{current_fork:?}")));
        }

        let (blobs_bundle, block_value) = build_payload(&mut payload, &context.storage)
            .map_err(|err| RpcErr::Internal(err.to_string()))?;
        let execution_payload = ExecutionPayload::from_block(payload);

        serde_json::to_value(self.build_response(execution_payload, blobs_bundle, block_value))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct GetPayloadV3Request(pub GetPayloadRequest);

impl From<GetPayloadV3Request> for RpcRequest {
    fn from(val: GetPayloadV3Request) -> Self {
        GetPayloadRequest::from(&val.0)
    }
}

impl RpcHandler for GetPayloadV3Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self(GetPayloadRequest::parse(
            params,
            GetPayloadRequestVersion::V3,
        )?))
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        GetPayloadRequest::handle(&self.0, context)
    }
}

pub struct GetPayloadV2Request(pub GetPayloadRequest);

impl From<GetPayloadV2Request> for RpcRequest {
    fn from(val: GetPayloadV2Request) -> Self {
        GetPayloadRequest::from(&val.0)
    }
}

impl RpcHandler for GetPayloadV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self(GetPayloadRequest::parse(
            params,
            GetPayloadRequestVersion::V2,
        )?))
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        GetPayloadRequest::handle(&self.0, context)
    }
}
