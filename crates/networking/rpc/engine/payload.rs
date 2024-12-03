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

pub struct NewPayloadRequest {}

impl NewPayloadRequest {
    fn handle(
        request: &dyn NewPayloadRequestImpl,
        context: RpcApiContext,
    ) -> Result<Value, RpcErr> {
        let storage = &context.storage;

        let block_hash = request.payload().block_hash;
        info!("Received new payload with block hash: {block_hash:#x}");

        let block = match request
            .payload()
            .clone()
            .into_block(request.parent_beacon_block_root())
        {
            Ok(block) => block,
            Err(error) => {
                let result = PayloadStatus::invalid_with_err(&error.to_string());
                return serde_json::to_value(result)
                    .map_err(|error| RpcErr::Internal(error.to_string()));
            }
        };

        // Payload Validation
        request.validate_execution_payload()?;

        // Check timestamp is post valid fork
        let chain_config = storage.get_chain_config()?;
        let current_fork = chain_config.get_fork(block.header.timestamp);
        if current_fork < request.valid_fork() {
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
        if request.are_blob_versioned_hashes_invalid(&block) {
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

trait NewPayloadRequestImpl {
    fn method(&self) -> String;
    fn payload(&self) -> &ExecutionPayload;
    fn parent_beacon_block_root(&self) -> Option<H256>;
    fn validate_execution_payload(&self) -> Result<(), RpcErr>;
    fn valid_fork(&self) -> Fork;
    fn are_blob_versioned_hashes_invalid(&self, block: &Block) -> bool;
}

pub struct NewPayloadV3Request {
    pub payload: ExecutionPayload,
    pub expected_blob_versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
}

impl NewPayloadRequestImpl for NewPayloadV3Request {
    fn method(&self) -> String {
        "engine_newPayloadV3".to_string()
    }

    fn payload(&self) -> &ExecutionPayload {
        &self.payload
    }

    fn parent_beacon_block_root(&self) -> Option<H256> {
        Some(self.parent_beacon_block_root)
    }

    fn valid_fork(&self) -> Fork {
        Fork::Cancun
    }

    fn are_blob_versioned_hashes_invalid(&self, block: &Block) -> bool {
        // Concatenate blob versioned hashes lists (tx.blob_versioned_hashes) of each blob transaction included in the payload, respecting the order of inclusion
        // and check that the resulting array matches expected_blob_versioned_hashes
        let blob_versioned_hashes: Vec<H256> = block
            .body
            .transactions
            .iter()
            .flat_map(|tx| tx.blob_versioned_hashes())
            .collect();
        self.expected_blob_versioned_hashes != blob_versioned_hashes
    }

    fn validate_execution_payload(&self) -> Result<(), RpcErr> {
        if self.payload().excess_blob_gas.is_none() {
            return Err(RpcErr::WrongParam("excess_blob_gas".to_string()));
        }
        if self.payload().blob_gas_used.is_none() {
            return Err(RpcErr::WrongParam("blob_gas_used".to_string()));
        }
        Ok(())
    }
}

impl From<NewPayloadV3Request> for RpcRequest {
    fn from(val: NewPayloadV3Request) -> Self {
        RpcRequest {
            method: val.method(),
            params: Some(vec![
                serde_json::json!(val.payload),
                serde_json::json!(val.expected_blob_versioned_hashes),
                serde_json::json!(val.parent_beacon_block_root),
            ]),
            ..Default::default()
        }
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
            payload: serde_json::from_value(params[0].clone())
                .map_err(|_| RpcErr::WrongParam("payload".to_string()))?,
            expected_blob_versioned_hashes: serde_json::from_value(params[1].clone())
                .map_err(|_| RpcErr::WrongParam("expected_blob_versioned_hashes".to_string()))?,
            parent_beacon_block_root: serde_json::from_value(params[2].clone())
                .map_err(|_| RpcErr::WrongParam("parent_beacon_block_root".to_string()))?,
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        NewPayloadRequest::handle(self, context)
    }
}

pub struct NewPayloadV2Request {
    pub payload: ExecutionPayload,
}

impl NewPayloadRequestImpl for NewPayloadV2Request {
    fn method(&self) -> String {
        "engine_newPayloadV2".to_string()
    }

    fn payload(&self) -> &ExecutionPayload {
        &self.payload
    }

    fn parent_beacon_block_root(&self) -> Option<H256> {
        None
    }

    fn valid_fork(&self) -> Fork {
        Fork::Shanghai
    }

    fn are_blob_versioned_hashes_invalid(&self, _block: &Block) -> bool {
        false
    }

    fn validate_execution_payload(&self) -> Result<(), RpcErr> {
        Ok(())
    }
}

impl From<NewPayloadV2Request> for RpcRequest {
    fn from(val: NewPayloadV2Request) -> Self {
        RpcRequest {
            method: val.method(),
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
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 params".to_owned()));
        }
        Ok(NewPayloadV2Request {
            payload: serde_json::from_value(params[0].clone())
                .map_err(|_| RpcErr::WrongParam("payload".to_string()))?,
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        NewPayloadRequest::handle(self, context)
    }
}

pub struct GetPayloadRequest {
    pub payload_id: u64,
}

impl GetPayloadRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetPayloadRequest, RpcErr> {
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
        Ok(GetPayloadRequest { payload_id })
    }

    fn from(val: &dyn GetPayloadRequestImpl) -> RpcRequest {
        RpcRequest {
            method: val.method(),
            params: Some(vec![serde_json::json!(U256::from(
                val.request().payload_id
            ))]),
            ..Default::default()
        }
    }

    fn handle(
        get_payload_request: &dyn GetPayloadRequestImpl,
        context: RpcApiContext,
    ) -> Result<Value, RpcErr> {
        let request = get_payload_request.request();
        info!("Requested payload with id: {:#018x}", request.payload_id);
        let Some(mut payload) = context.storage.get_payload(request.payload_id)? else {
            return Err(RpcErr::UnknownPayload(format!(
                "Payload with id {:#018x} not found",
                request.payload_id
            )));
        };
        // Check timestamp matches valid fork
        let chain_config = &context.storage.get_chain_config()?;
        let current_fork = chain_config.get_fork(payload.header.timestamp);
        info!("Current Fork: {:?}", current_fork);
        if current_fork != get_payload_request.valid_fork() {
            return Err(RpcErr::UnsuportedFork(format!("{current_fork:?}")));
        }

        let (blobs_bundle, block_value) = build_payload(&mut payload, &context.storage)
            .map_err(|err| RpcErr::Internal(err.to_string()))?;
        let execution_payload = ExecutionPayload::from_block(payload);

        serde_json::to_value(get_payload_request.build_response(
            execution_payload,
            blobs_bundle,
            block_value,
        ))
        .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
trait GetPayloadRequestImpl {
    fn method(&self) -> String;
    fn request(&self) -> &GetPayloadRequest;
    fn valid_fork(&self) -> Fork;
    fn build_response(
        &self,
        execution_payload: ExecutionPayload,
        blobs_bundle: BlobsBundle,
        block_value: U256,
    ) -> ExecutionPayloadResponse;
}

pub struct GetPayloadV3Request(pub GetPayloadRequest);

impl GetPayloadRequestImpl for GetPayloadV3Request {
    fn method(&self) -> String {
        "engine_getPayloadV3".to_string()
    }

    fn request(&self) -> &GetPayloadRequest {
        &self.0
    }

    fn valid_fork(&self) -> Fork {
        Fork::Cancun
    }

    fn build_response(
        &self,
        execution_payload: ExecutionPayload,
        blobs_bundle: BlobsBundle,
        block_value: U256,
    ) -> ExecutionPayloadResponse {
        ExecutionPayloadResponse {
            execution_payload,
            block_value,
            blobs_bundle: Some(blobs_bundle),
            should_override_builder: Some(false),
        }
    }
}

impl From<GetPayloadV3Request> for RpcRequest {
    fn from(val: GetPayloadV3Request) -> Self {
        GetPayloadRequest::from(&val)
    }
}

impl RpcHandler for GetPayloadV3Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self(GetPayloadRequest::parse(params)?))
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        GetPayloadRequest::handle(self, context)
    }
}

pub struct GetPayloadV2Request(pub GetPayloadRequest);

impl GetPayloadRequestImpl for GetPayloadV2Request {
    fn method(&self) -> String {
        "engine_getPayloadV2".to_string()
    }

    fn request(&self) -> &GetPayloadRequest {
        &self.0
    }

    fn valid_fork(&self) -> Fork {
        Fork::Shanghai
    }

    fn build_response(
        &self,
        execution_payload: ExecutionPayload,
        _blobs_bundle: BlobsBundle,
        block_value: U256,
    ) -> ExecutionPayloadResponse {
        ExecutionPayloadResponse {
            execution_payload,
            block_value,
            blobs_bundle: None,
            should_override_builder: None,
        }
    }
}

impl From<GetPayloadV2Request> for RpcRequest {
    fn from(val: GetPayloadV2Request) -> Self {
        GetPayloadRequest::from(&val)
    }
}

impl RpcHandler for GetPayloadV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self(GetPayloadRequest::parse(params)?))
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        GetPayloadRequest::handle(self, context)
    }
}
