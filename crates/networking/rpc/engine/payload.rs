use ethrex_blockchain::error::ChainError;
use ethrex_common::types::payload::PayloadBundle;
use ethrex_common::types::requests::{compute_requests_hash, EncodedRequests};
use ethrex_common::types::{Block, BlockBody, BlockHash, BlockNumber, Fork};
use ethrex_common::{H256, U256};
use serde_json::Value;
use tracing::{debug, error, info, warn};

use crate::types::payload::{
    ExecutionPayload, ExecutionPayloadBody, ExecutionPayloadResponse, PayloadStatus,
};
use crate::utils::{parse_json_hex, RpcRequest};
use crate::{RpcApiContext, RpcErr, RpcHandler, SyncStatus};

// Must support rquest sizes of at least 32 blocks
// Chosen an arbitrary x4 value
// -> https://github.com/ethereum/execution-apis/blob/main/src/engine/shanghai.md#specification-3
const GET_PAYLOAD_BODIES_REQUEST_MAX_SIZE: usize = 128;

// NewPayload V1-V2-V3 implementations
pub struct NewPayloadV1Request {
    pub payload: ExecutionPayload,
}

impl RpcHandler for NewPayloadV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(NewPayloadV1Request {
            payload: parse_execution_payload(params)?,
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        validate_execution_payload_v1(&self.payload)?;
        handle_new_payload_v1_v2(&self.payload, context)
    }
}

pub struct NewPayloadV2Request {
    pub payload: ExecutionPayload,
}

impl RpcHandler for NewPayloadV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(NewPayloadV2Request {
            payload: parse_execution_payload(params)?,
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let chain_config = &context.storage.get_chain_config()?;
        if chain_config.is_shanghai_activated(self.payload.timestamp) {
            validate_execution_payload_v2(&self.payload)?;
        } else {
            // Behave as a v1
            validate_execution_payload_v1(&self.payload)?;
        }

        handle_new_payload_v1_v2(&self.payload, context)
    }
}

pub struct NewPayloadV3Request {
    pub payload: ExecutionPayload,
    pub expected_blob_versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
}

impl From<NewPayloadV3Request> for RpcRequest {
    fn from(val: NewPayloadV3Request) -> Self {
        RpcRequest {
            method: "engine_newPayloadV3".to_string(),
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

    #[cfg(feature = "based")]
    async fn relay_to_gateway_or_fallback(
        req: &RpcRequest,
        context: RpcApiContext,
    ) -> Result<Value, RpcErr> {
        info!("Relaying engine_getPayloadV3 to gateway");

        let request = Self::parse(&req.params)?;

        let gateway_auth_client = context.gateway_auth_client.clone();

        let gateway_request = gateway_auth_client.engine_new_payload_v3(
            request.payload,
            request.expected_blob_versioned_hashes,
            request.parent_beacon_block_root,
        );

        let client_response = Self::call(req, context);

        let gateway_response = gateway_request
            .await
            .map_err(|err| {
                RpcErr::Internal(format!(
                    "Could not relay engine_newPayloadV3 to gateway: {err}",
                ))
            })
            .and_then(|response| {
                serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
            });

        if gateway_response.is_err() {
            warn!(error = ?gateway_response, "Gateway engine_newPayloadV3 failed, falling back to local node");
        } else {
            info!("Successfully relayed engine_newPayloadV3 to gateway");
        }

        gateway_response.or(client_response)
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block =
            get_block_from_payload(&self.payload, Some(self.parent_beacon_block_root), None)?;
        validate_fork(&block, Fork::Cancun, &context)?;
        validate_execution_payload_v3(&self.payload)?;
        let payload_status = handle_new_payload_v3(
            &self.payload,
            context,
            block,
            self.expected_blob_versioned_hashes.clone(),
        )?;
        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
pub struct NewPayloadV4Request {
    pub payload: ExecutionPayload,
    pub expected_blob_versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
    pub execution_requests: Vec<EncodedRequests>,
}

impl RpcHandler for NewPayloadV4Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 4 {
            return Err(RpcErr::BadParams("Expected 4 params".to_owned()));
        }
        Ok(NewPayloadV4Request {
            payload: serde_json::from_value(params[0].clone())
                .map_err(|_| RpcErr::WrongParam("payload".to_string()))?,
            expected_blob_versioned_hashes: serde_json::from_value(params[1].clone())
                .map_err(|_| RpcErr::WrongParam("expected_blob_versioned_hashes".to_string()))?,
            parent_beacon_block_root: serde_json::from_value(params[2].clone())
                .map_err(|_| RpcErr::WrongParam("parent_beacon_block_root".to_string()))?,
            execution_requests: serde_json::from_value(params[3].clone())
                .map_err(|_| RpcErr::WrongParam("execution_requests".to_string()))?,
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // validate the received requests
        validate_execution_requests(&self.execution_requests)?;

        let requests_hash = compute_requests_hash(&self.execution_requests);
        let block = get_block_from_payload(
            &self.payload,
            Some(self.parent_beacon_block_root),
            Some(requests_hash),
        )?;

        let chain_config = context.storage.get_chain_config()?;

        if !chain_config.is_prague_activated(block.header.timestamp) {
            return Err(RpcErr::UnsuportedFork(format!(
                "{:?}",
                chain_config.get_fork(block.header.timestamp)
            )));
        }
        // We use v3 since the execution payload remains the same.
        validate_execution_payload_v3(&self.payload)?;
        let payload_status = handle_new_payload_v3(
            &self.payload,
            context,
            block,
            self.expected_blob_versioned_hashes.clone(),
        )?;
        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

// GetPayload V1-V2-V3 implementations
pub struct GetPayloadV1Request {
    pub payload_id: u64,
}

impl RpcHandler for GetPayloadV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let payload_id = parse_get_payload_request(params)?;
        Ok(Self { payload_id })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload = get_payload(self.payload_id, &context)?;
        // NOTE: This validation is actually not required to run Hive tests. Not sure if it's
        // necessary
        validate_payload_v1_v2(&payload.block, &context)?;
        let payload_bundle = build_payload_if_necessary(self.payload_id, payload, context)?;

        let response = ExecutionPayload::from_block(payload_bundle.block);

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct GetPayloadV2Request {
    pub payload_id: u64,
}

impl RpcHandler for GetPayloadV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let payload_id = parse_get_payload_request(params)?;
        Ok(Self { payload_id })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload = get_payload(self.payload_id, &context)?;
        validate_payload_v1_v2(&payload.block, &context)?;
        let payload_bundle = build_payload_if_necessary(self.payload_id, payload, context)?;

        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(payload_bundle.block),
            block_value: payload_bundle.block_value,
            blobs_bundle: None,
            should_override_builder: None,
            execution_requests: None,
        };

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct GetPayloadV3Request {
    pub payload_id: u64,
}

impl From<GetPayloadV3Request> for RpcRequest {
    fn from(val: GetPayloadV3Request) -> Self {
        RpcRequest {
            method: "engine_getPayloadV3".to_string(),
            params: Some(vec![serde_json::json!(U256::from(val.payload_id))]),
            ..Default::default()
        }
    }
}

impl RpcHandler for GetPayloadV3Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let payload_id = parse_get_payload_request(params)?;
        Ok(Self { payload_id })
    }

    #[cfg(feature = "based")]
    async fn relay_to_gateway_or_fallback(
        req: &RpcRequest,
        context: RpcApiContext,
    ) -> Result<Value, RpcErr> {
        info!("Relaying engine_getPayloadV3 to gateway");

        let request = Self::parse(&req.params)?;

        let gateway_auth_client = context.gateway_auth_client.clone();

        let gateway_request = gateway_auth_client.engine_get_payload_v3(request.payload_id);

        let client_response = Self::call(req, context);

        let gateway_response = gateway_request
            .await
            .map_err(|err| {
                RpcErr::Internal(format!(
                    "Could not relay engine_getPayloadV3 to gateway: {err}",
                ))
            })
            .and_then(|response| {
                serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
            });

        if gateway_response.is_err() {
            warn!(error = ?gateway_response, "Gateway engine_getPayloadV3 failed, falling back to local node");
        } else {
            info!("Successfully relayed engine_getPayloadV3 to gateway");
        }

        gateway_response.or(client_response)
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload = get_payload(self.payload_id, &context)?;
        validate_fork(&payload.block, Fork::Cancun, &context)?;
        let payload_bundle = build_payload_if_necessary(self.payload_id, payload, context)?;

        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(payload_bundle.block),
            block_value: payload_bundle.block_value,
            blobs_bundle: Some(payload_bundle.blobs_bundle),
            should_override_builder: Some(false),
            execution_requests: None,
        };

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct GetPayloadV4Request {
    pub payload_id: u64,
}

impl From<GetPayloadV4Request> for RpcRequest {
    fn from(val: GetPayloadV4Request) -> Self {
        RpcRequest {
            method: "engine_getPayloadV4".to_string(),
            params: Some(vec![serde_json::json!(U256::from(val.payload_id))]),
            ..Default::default()
        }
    }
}

impl RpcHandler for GetPayloadV4Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let payload_id = parse_get_payload_request(params)?;
        Ok(Self { payload_id })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload = get_payload(self.payload_id, &context)?;
        let chain_config = &context.storage.get_chain_config()?;

        if !chain_config.is_prague_activated(payload.block.header.timestamp) {
            return Err(RpcErr::UnsuportedFork(format!(
                "{:?}",
                chain_config.get_fork(payload.block.header.timestamp)
            )));
        }

        let payload_bundle = build_payload_if_necessary(self.payload_id, payload, context)?;

        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(payload_bundle.block),
            block_value: payload_bundle.block_value,
            blobs_bundle: Some(payload_bundle.blobs_bundle),
            should_override_builder: Some(false),
            execution_requests: Some(
                payload_bundle
                    .requests
                    .into_iter()
                    .filter(|r| !r.is_empty())
                    .collect(),
            ),
        };

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct GetPayloadBodiesByHashV1Request {
    pub hashes: Vec<BlockHash>,
}

impl RpcHandler for GetPayloadBodiesByHashV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };

        Ok(GetPayloadBodiesByHashV1Request {
            hashes: serde_json::from_value(params[0].clone())?,
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        if self.hashes.len() >= GET_PAYLOAD_BODIES_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }
        let bodies = self
            .hashes
            .iter()
            .map(|hash| context.storage.get_block_body_by_hash(*hash))
            .collect::<Result<Vec<Option<BlockBody>>, _>>()?;
        build_payload_body_response(bodies)
    }
}

pub struct GetPayloadBodiesByRangeV1Request {
    start: BlockNumber,
    count: u64,
}

impl RpcHandler for GetPayloadBodiesByRangeV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        let start = parse_json_hex(&params[0]).map_err(|_| RpcErr::BadHexFormat(0))?;
        let count = parse_json_hex(&params[1]).map_err(|_| RpcErr::BadHexFormat(1))?;
        if start < 1 {
            return Err(RpcErr::WrongParam("start".to_owned()));
        }
        if count < 1 {
            return Err(RpcErr::WrongParam("count".to_owned()));
        }
        Ok(GetPayloadBodiesByRangeV1Request { start, count })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        if self.count as usize >= GET_PAYLOAD_BODIES_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }
        let latest_block_number = context.storage.get_latest_block_number()?;
        let last = latest_block_number.min(self.start + self.count - 1);
        let bodies = (self.start..=last)
            .map(|block_num| context.storage.get_block_body(block_num))
            .collect::<Result<Vec<Option<BlockBody>>, _>>()?;
        build_payload_body_response(bodies)
    }
}

fn build_payload_body_response(bodies: Vec<Option<BlockBody>>) -> Result<Value, RpcErr> {
    let response: Vec<Option<ExecutionPayloadBody>> = bodies
        .into_iter()
        .map(|body| body.map(Into::into))
        .collect();
    serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
}

fn parse_execution_payload(params: &Option<Vec<Value>>) -> Result<ExecutionPayload, RpcErr> {
    let params = params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
    if params.len() != 1 {
        return Err(RpcErr::BadParams("Expected 1 params".to_owned()));
    }
    serde_json::from_value(params[0].clone()).map_err(|_| RpcErr::WrongParam("payload".to_string()))
}

fn validate_execution_payload_v1(payload: &ExecutionPayload) -> Result<(), RpcErr> {
    // Validate that only the required arguments are present
    if payload.withdrawals.is_some() {
        return Err(RpcErr::WrongParam("withdrawals".to_string()));
    }
    if payload.blob_gas_used.is_some() {
        return Err(RpcErr::WrongParam("blob_gas_used".to_string()));
    }
    if payload.excess_blob_gas.is_some() {
        return Err(RpcErr::WrongParam("excess_blob_gas".to_string()));
    }

    Ok(())
}

fn validate_execution_payload_v2(payload: &ExecutionPayload) -> Result<(), RpcErr> {
    // Validate that only the required arguments are present
    if payload.withdrawals.is_none() {
        return Err(RpcErr::WrongParam("withdrawals".to_string()));
    }
    if payload.blob_gas_used.is_some() {
        return Err(RpcErr::WrongParam("blob_gas_used".to_string()));
    }
    if payload.excess_blob_gas.is_some() {
        return Err(RpcErr::WrongParam("excess_blob_gas".to_string()));
    }

    Ok(())
}

fn validate_execution_payload_v3(payload: &ExecutionPayload) -> Result<(), RpcErr> {
    // Validate that only the required arguments are present
    if payload.withdrawals.is_none() {
        return Err(RpcErr::WrongParam("withdrawals".to_string()));
    }
    if payload.blob_gas_used.is_none() {
        return Err(RpcErr::WrongParam("blob_gas_used".to_string()));
    }
    if payload.excess_blob_gas.is_none() {
        return Err(RpcErr::WrongParam("excess_blob_gas".to_string()));
    }

    Ok(())
}

fn validate_payload_v1_v2(block: &Block, context: &RpcApiContext) -> Result<(), RpcErr> {
    let chain_config = &context.storage.get_chain_config()?;
    if chain_config.is_cancun_activated(block.header.timestamp) {
        return Err(RpcErr::UnsuportedFork(
            "Cancun payload received".to_string(),
        ));
    }
    Ok(())
}

fn handle_new_payload_v1_v2(
    payload: &ExecutionPayload,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    let block = get_block_from_payload(payload, None, None)?;
    let payload_status = match context.sync_status()? {
        SyncStatus::Active | SyncStatus::Pending => PayloadStatus::syncing(),
        SyncStatus::Inactive => {
            // Check if the block has already been invalidated
            let invalid_ancestors = match context.syncer.try_lock() {
                Ok(syncer) => syncer.invalid_ancestors.clone(),
                Err(_) => return Err(RpcErr::Internal("Internal error".into())),
            };
            if let Some(latest_valid_hash) = invalid_ancestors.get(&block.hash()) {
                PayloadStatus::invalid_with(
                    *latest_valid_hash,
                    "Header has been previously invalidated.".into(),
                )
            } else if let Err(RpcErr::Internal(error_msg)) = validate_block_hash(payload, &block) {
                PayloadStatus::invalid_with_err(&error_msg)
            } else {
                execute_payload(&block, &context)?
            }
        }
    };
    serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
}

fn handle_new_payload_v3(
    payload: &ExecutionPayload,
    context: RpcApiContext,
    block: Block,
    expected_blob_versioned_hashes: Vec<H256>,
) -> Result<PayloadStatus, RpcErr> {
    // Ignore incoming
    // match dbg!(context.sync_status()?) {
    // SyncStatus::Active | SyncStatus::Pending => {
    //     dbg!("Sync status");
    //     Ok(PayloadStatus::syncing())
    // }
    // SyncStatus::Inactive => {
    // _ => {
    if let Err(RpcErr::Internal(error_msg)) = validate_block_hash(payload, &block) {
        dbg!("HAY UN ERROR EN LA VALIDACION DEL SERVER");
        return Ok(PayloadStatus::invalid_with_err(&error_msg));
    }
    let blob_versioned_hashes: Vec<H256> = block
        .body
        .transactions
        .iter()
        .flat_map(|tx| tx.blob_versioned_hashes())
        .collect();

    if expected_blob_versioned_hashes != blob_versioned_hashes {
        return Ok(PayloadStatus::invalid_with_err(
            "Invalid blob_versioned_hashes",
        ));
    }
    dbg!("esta todo bien loco, no pasa nada");
    dbg!(execute_payload(&block, &context))
    // }
    // }
}

// Elements of the list MUST be ordered by request_type in ascending order.
// Elements with empty request_data MUST be excluded from the list.
fn validate_execution_requests(execution_requests: &[EncodedRequests]) -> Result<(), RpcErr> {
    let mut last_type: i32 = -1;
    for requests in execution_requests {
        if requests.0.len() < 2 {
            return Err(RpcErr::WrongParam("Empty requests data.".to_string()));
        }
        let request_type = requests.0[0] as i32;
        if last_type >= request_type {
            return Err(RpcErr::WrongParam("Invalid requests order.".to_string()));
        }
        last_type = request_type;
    }
    Ok(())
}

fn get_block_from_payload(
    payload: &ExecutionPayload,
    parent_beacon_block_root: Option<H256>,
    requests_hash: Option<H256>,
) -> Result<Block, RpcErr> {
    let block_hash = payload.block_hash;
    info!("Received new payload with block hash: {block_hash:#x}");

    payload
        .clone()
        .into_block(parent_beacon_block_root, requests_hash)
        .map_err(|error| RpcErr::Internal(error.to_string()))
}

fn validate_block_hash(payload: &ExecutionPayload, block: &Block) -> Result<(), RpcErr> {
    let block_hash = payload.block_hash;
    let actual_block_hash = block.hash();
    if block_hash != actual_block_hash {
        return Err(RpcErr::Internal(format!(
            "Invalid block hash. Expected {actual_block_hash:#x}, got {block_hash:#x}"
        )));
    }
    debug!("Block hash {block_hash} is valid");
    Ok(())
}

fn execute_payload(block: &Block, context: &RpcApiContext) -> Result<PayloadStatus, RpcErr> {
    let block_hash = block.hash();
    let storage = &context.storage;
    // Return the valid message directly if we have it.
    if storage.get_block_by_hash(block_hash)?.is_some() {
        return Ok(PayloadStatus::valid_with_hash(block_hash));
    }

    // Execute and store the block
    info!("Executing payload with block hash: {block_hash:#x}");
    let latest_valid_hash =
        context
            .storage
            .get_latest_canonical_block_hash()?
            .ok_or(RpcErr::Internal(
                "Missing latest canonical block".to_owned(),
            ))?;

    // adds a bad block as a bad ancestor so we can catch it on fork_choice as well
    let add_block_to_invalid_ancestor = || {
        let lock = context.syncer.try_lock();
        if let Ok(mut syncer) = lock {
            syncer
                .invalid_ancestors
                .insert(block_hash, latest_valid_hash);
        };
    };

    match context.blockchain.add_block(block) {
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
            add_block_to_invalid_ancestor();
            Ok(PayloadStatus::invalid_with(
                latest_valid_hash,
                error.to_string(),
            ))
        }
        Err(ChainError::EvmError(error)) => {
            warn!("Error executing block: {error}");
            add_block_to_invalid_ancestor();
            Ok(PayloadStatus::invalid_with(
                latest_valid_hash,
                error.to_string(),
            ))
        }
        Err(ChainError::StoreError(error)) => {
            warn!("Error storing block: {error}");
            Err(RpcErr::Internal(error.to_string()))
        }
        Err(ChainError::Custom(e)) => {
            error!("{e} for block {block_hash}");
            Err(RpcErr::Internal(e.to_string()))
        }
        Ok(()) => {
            info!("Block with hash {block_hash} executed and added to storage succesfully");
            Ok(PayloadStatus::valid_with_hash(block_hash))
        }
    }
}

fn parse_get_payload_request(params: &Option<Vec<Value>>) -> Result<u64, RpcErr> {
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
    Ok(payload_id)
}

fn get_payload(payload_id: u64, context: &RpcApiContext) -> Result<PayloadBundle, RpcErr> {
    info!("Requested payload with id: {:#018x}", payload_id);
    let Some(payload) = context.storage.get_payload(payload_id)? else {
        return Err(RpcErr::UnknownPayload(format!(
            "Payload with id {:#018x} not found",
            payload_id
        )));
    };
    Ok(payload)
}

fn validate_fork(block: &Block, fork: Fork, context: &RpcApiContext) -> Result<(), RpcErr> {
    // Check timestamp matches valid fork
    let chain_config = &context.storage.get_chain_config()?;
    let current_fork = chain_config.get_fork(block.header.timestamp);

    if current_fork != fork {
        return Err(RpcErr::UnsuportedFork(format!("{current_fork:?}")));
    }
    Ok(())
}

fn build_payload_if_necessary(
    payload_id: u64,
    mut payload: PayloadBundle,
    context: RpcApiContext,
) -> Result<PayloadBundle, RpcErr> {
    if payload.completed {
        Ok(payload)
    } else {
        let (blobs_bundle, requests, block_value) = {
            context
                .blockchain
                .build_payload(&mut payload.block)
                .map_err(|err| RpcErr::Internal(err.to_string()))?
        };

        let new_payload = PayloadBundle {
            block: payload.block,
            block_value,
            blobs_bundle,
            requests,
            completed: true,
        };

        context
            .storage
            .update_payload(payload_id, new_payload.clone())?;

        Ok(new_payload)
    }
}
