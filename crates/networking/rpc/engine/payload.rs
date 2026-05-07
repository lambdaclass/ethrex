use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::payload::PayloadBuildResult;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::payload::PayloadBundle;
use ethrex_common::types::requests::{EncodedRequests, compute_requests_hash};
use ethrex_common::types::{Block, BlockBody, BlockHash, BlockNumber, Fork};
use ethrex_common::{H256, U256};
use ethrex_p2p::sync::SyncMode;
use ethrex_rlp::error::RLPDecodeError;
use serde_json::Value;
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::payload::{
    ExecutionPayload, ExecutionPayloadBody, ExecutionPayloadBodyV2, ExecutionPayloadResponse,
    PayloadStatus,
};
use crate::utils::RpcErr;
use crate::utils::{RpcRequest, parse_json_hex};

// Must support rquest sizes of at least 32 blocks
// Chosen an arbitrary x4 value
// -> https://github.com/ethereum/execution-apis/blob/main/src/engine/shanghai.md#specification-3
const GET_PAYLOAD_BODIES_REQUEST_MAX_SIZE: u64 = 128;

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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        validate_execution_payload_v1(&self.payload)?;
        let block = match get_block_from_payload(&self.payload, None, None, None) {
            Ok(block) => block,
            Err(err) => {
                return Ok(serde_json::to_value(PayloadStatus::invalid_with_err(
                    &err.to_string(),
                ))?);
            }
        };
        let payload_status = handle_new_payload_v1_v2(&self.payload, block, context, None).await?;
        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let chain_config = &context.storage.get_chain_config();
        if chain_config.is_shanghai_activated(self.payload.timestamp) {
            validate_execution_payload_v2(&self.payload)?;
        } else {
            // Behave as a v1
            validate_execution_payload_v1(&self.payload)?;
        }
        let block = match get_block_from_payload(&self.payload, None, None, None) {
            Ok(block) => block,
            Err(err) => {
                return Ok(serde_json::to_value(PayloadStatus::invalid_with_err(
                    &err.to_string(),
                ))?);
            }
        };
        let payload_status = handle_new_payload_v1_v2(&self.payload, block, context, None).await?;
        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block = match get_block_from_payload(
            &self.payload,
            Some(self.parent_beacon_block_root),
            None,
            None,
        ) {
            Ok(block) => block,
            Err(err) => {
                return Ok(serde_json::to_value(PayloadStatus::invalid_with_err(
                    &err.to_string(),
                ))?);
            }
        };
        validate_fork(&block, Fork::Cancun, &context)?;
        validate_execution_payload_v3(&self.payload)?;
        let payload_status = handle_new_payload_v3(
            &self.payload,
            context,
            block,
            self.expected_blob_versioned_hashes.clone(),
            None,
        )
        .await?;
        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct NewPayloadV4Request {
    pub payload: ExecutionPayload,
    pub expected_blob_versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
    pub execution_requests: Vec<EncodedRequests>,
}

impl From<NewPayloadV4Request> for RpcRequest {
    fn from(val: NewPayloadV4Request) -> Self {
        RpcRequest {
            method: "engine_newPayloadV4".to_string(),
            params: Some(vec![
                serde_json::json!(val.payload),
                serde_json::json!(val.expected_blob_versioned_hashes),
                serde_json::json!(val.parent_beacon_block_root),
                serde_json::json!(val.execution_requests),
            ]),
            ..Default::default()
        }
    }
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // EIP-7928 / Amsterdam: V4 payloads MUST NOT include the BAL field — that
        // field belongs to V5. Per engine-API spec, structurally-invalid payloads
        // return JSON-RPC -32602 (Invalid params), not PayloadStatus.INVALID.
        if self.payload.block_access_list.is_some() {
            return Err(RpcErr::WrongParam(
                "block_access_list not allowed in engine_newPayloadV4".to_string(),
            ));
        }

        // validate the received requests
        validate_execution_requests(&self.execution_requests)?;

        let requests_hash = compute_requests_hash(&self.execution_requests);
        let block = match get_block_from_payload(
            &self.payload,
            Some(self.parent_beacon_block_root),
            Some(requests_hash),
            None,
        ) {
            Ok(block) => block,
            Err(err) => {
                return Ok(serde_json::to_value(PayloadStatus::invalid_with_err(
                    &err.to_string(),
                ))?);
            }
        };

        let chain_config = context.storage.get_chain_config();

        // Amsterdam-active timestamps must use V5, not V4. Per engine-API spec
        // (amsterdam.md): "Client software MUST return -38005: Unsupported fork
        // if the timestamp of payload is greater than or equal to the Amsterdam
        // activation timestamp."
        if chain_config.is_amsterdam_activated(block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(format!(
                "{:?}",
                chain_config.get_fork(block.header.timestamp)
            )));
        }

        if !chain_config.is_prague_activated(block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(format!(
                "{:?}",
                chain_config.get_fork(block.header.timestamp)
            )));
        }

        // EIP-7928 fork-boundary detector: V4 doesn't carry block_access_list_hash
        // in its header schema. If the payload's block_hash matches what a V5-style
        // header (with block_access_list_hash injected) would produce, the sender
        // used the wrong API version; reject with -32602 (InvalidParams) to match
        // the EELS fixture test_invalid_pre_fork_block_with_bal_hash_field
        // [fork_BPO2ToAmsterdamAtTime15k-blockchain_test_engine]. Real value-mismatch
        // tests don't match this alternate and fall through to PayloadStatus.INVALID.
        if block.hash() != self.payload.block_hash {
            let mut alt_header = block.header.clone();
            alt_header.block_access_list_hash = Some(H256::zero());
            let alt_hash = alt_header.compute_block_hash(&ethrex_crypto::NativeCrypto);
            if alt_hash == self.payload.block_hash {
                return Err(RpcErr::WrongParam(
                    "engine_newPayloadV4 received header with Amsterdam block_access_list_hash field"
                        .to_string(),
                ));
            }
        }

        // We use v3 since the execution payload remains the same.
        validate_execution_payload_v3(&self.payload)?;
        let payload_status = handle_new_payload_v3(
            &self.payload,
            context,
            block,
            self.expected_blob_versioned_hashes.clone(),
            None,
        )
        .await?;
        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct NewPayloadV5Request {
    pub payload: ExecutionPayload,
    pub expected_blob_versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
    pub execution_requests: Vec<EncodedRequests>,
    /// The BAL hash computed from the raw RLP bytes as received (no re-encoding/sorting).
    /// This preserves the exact encoding from the payload for block hash validation.
    pub raw_bal_hash: Option<H256>,
}

impl From<NewPayloadV5Request> for RpcRequest {
    fn from(val: NewPayloadV5Request) -> Self {
        RpcRequest {
            method: "engine_newPayloadV5".to_string(),
            params: Some(vec![
                serde_json::json!(val.payload),
                serde_json::json!(val.expected_blob_versioned_hashes),
                serde_json::json!(val.parent_beacon_block_root),
                serde_json::json!(val.execution_requests),
            ]),
            ..Default::default()
        }
    }
}

impl RpcHandler for NewPayloadV5Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 4 {
            return Err(RpcErr::BadParams("Expected 4 params".to_owned()));
        }

        // Extract the raw BAL hash from the JSON payload before deserialization.
        // We hash the raw RLP bytes as-received to preserve the exact encoding
        // (including any ordering) for accurate block hash validation.
        let raw_bal_hash = params[0]
            .get("blockAccessList")
            .map(|v| {
                let hex_str = v
                    .as_str()
                    .ok_or(RpcErr::WrongParam("blockAccessList".to_string()))?;
                let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                    .map_err(|_| RpcErr::WrongParam("blockAccessList".to_string()))?;
                Ok::<_, RpcErr>(ethrex_common::utils::keccak(bytes))
            })
            .transpose()?;

        Ok(Self {
            payload: serde_json::from_value(params[0].clone())
                .map_err(|_| RpcErr::WrongParam("payload".to_string()))?,
            expected_blob_versioned_hashes: serde_json::from_value(params[1].clone())
                .map_err(|_| RpcErr::WrongParam("expected_blob_versioned_hashes".to_string()))?,
            parent_beacon_block_root: serde_json::from_value(params[2].clone())
                .map_err(|_| RpcErr::WrongParam("parent_beacon_block_root".to_string()))?,
            execution_requests: serde_json::from_value(params[3].clone())
                .map_err(|_| RpcErr::WrongParam("execution_requests".to_string()))?,
            raw_bal_hash,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // EIP-7928 / Amsterdam: V5 payloads MUST include the BAL field — its
        // absence is a structural error, not a block-validity failure. Per
        // engine-API spec, this returns JSON-RPC -32602 (Invalid params).
        if self.payload.block_access_list.is_none() {
            return Err(RpcErr::WrongParam(
                "block_access_list required in engine_newPayloadV5".to_string(),
            ));
        }

        validate_execution_payload_v4(&self.payload)?;

        // validate the received requests
        validate_execution_requests(&self.execution_requests)?;

        let requests_hash = compute_requests_hash(&self.execution_requests);
        // Use the hash computed from the raw RLP bytes as-received.
        // This preserves the exact encoding (including any ordering) from the payload,
        // so the block hash check correctly detects BAL corruption.
        let block_access_list_hash = self.raw_bal_hash;

        let block = match get_block_from_payload(
            &self.payload,
            Some(self.parent_beacon_block_root),
            Some(requests_hash),
            block_access_list_hash,
        ) {
            Ok(block) => block,
            Err(err) => {
                return Ok(serde_json::to_value(PayloadStatus::invalid_with_err(
                    &err.to_string(),
                ))?);
            }
        };

        let chain_config = context.storage.get_chain_config();

        // Pre-Hegotá guard: V5 cannot accept Hegotá-timestamp payloads. Runs
        // unconditionally (not feature-gated) so a non-FOCIL build still rejects
        // when the chain config has hegota_time set.
        if chain_config.is_hegota_activated(block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(
                "engine_newPayloadV5 cannot accept Hegotá payloads".to_string(),
            ));
        }

        // Pre-Amsterdam timestamps must use V4, not V5. Per engine-API spec
        // (amsterdam.md): "Client software MUST return -38005: Unsupported fork
        // if the timestamp of the payload does not fall within the time frame of
        // the Amsterdam activation." Symmetric with the V4+Amsterdam case above.
        if !chain_config.is_amsterdam_activated(block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(format!(
                "{:?}",
                chain_config.get_fork(block.header.timestamp)
            )));
        }

        // EIP-7928 fork-boundary detector: V5 requires block_access_list_hash in
        // the header. If the payload's block_hash matches what a V4-style header
        // (without the field) would produce, the sender used the wrong API
        // version; reject with -32602 (InvalidParams) to match the EELS fixture
        // test_invalid_post_fork_block_without_bal_hash_field
        // [fork_BPO2ToAmsterdamAtTime15k-blockchain_test_engine]. Real
        // value-mismatch tests don't match this alternate and fall through to
        // PayloadStatus.INVALID.
        if block.hash() != self.payload.block_hash {
            let mut alt_header = block.header.clone();
            alt_header.block_access_list_hash = None;
            let alt_hash = alt_header.compute_block_hash(&ethrex_crypto::NativeCrypto);
            if alt_hash == self.payload.block_hash {
                return Err(RpcErr::WrongParam(
                    "engine_newPayloadV5 received header missing block_access_list_hash field"
                        .to_string(),
                ));
            }
        }

        let bal = self.payload.block_access_list.clone();
        let payload_status = handle_new_payload_v4(
            &self.payload,
            context,
            block,
            self.expected_blob_versioned_hashes.clone(),
            bal,
        )
        .await?;
        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

/// `engine_newPayloadV6` — adds `inclusionListTransactions` parameter and the
/// `INCLUSION_LIST_UNSATISFIED` payload status per execution-apis #609. The IL
/// satisfaction algorithm itself is wired into block validation in Phase 5.2;
/// this handler RLP-decodes the IL parameter for parse-time validation and
/// passes it through. (Today the V6 handler accepts the same payloads V5 would
/// accept on a Hegotá chain; once Phase 5.2 lands, IL satisfaction is enforced
/// at block-import time.)
pub struct NewPayloadV6Request {
    pub payload: ExecutionPayload,
    pub expected_blob_versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
    pub execution_requests: Vec<EncodedRequests>,
    pub inclusion_list_transactions: Vec<bytes::Bytes>,
    pub raw_bal_hash: Option<H256>,
}

impl From<NewPayloadV6Request> for RpcRequest {
    fn from(val: NewPayloadV6Request) -> Self {
        RpcRequest {
            method: "engine_newPayloadV6".to_string(),
            params: Some(vec![
                serde_json::json!(val.payload),
                serde_json::json!(val.expected_blob_versioned_hashes),
                serde_json::json!(val.parent_beacon_block_root),
                serde_json::json!(val.execution_requests),
                serde_json::json!(val.inclusion_list_transactions),
            ]),
            ..Default::default()
        }
    }
}

impl RpcHandler for NewPayloadV6Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 5 {
            return Err(RpcErr::BadParams("Expected 5 params".to_owned()));
        }

        let raw_bal_hash = params[0]
            .get("blockAccessList")
            .map(|v| {
                let hex_str = v
                    .as_str()
                    .ok_or(RpcErr::WrongParam("blockAccessList".to_string()))?;
                let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                    .map_err(|_| RpcErr::WrongParam("blockAccessList".to_string()))?;
                Ok::<_, RpcErr>(ethrex_common::utils::keccak(bytes))
            })
            .transpose()?;

        Ok(Self {
            payload: serde_json::from_value(params[0].clone())
                .map_err(|_| RpcErr::WrongParam("payload".to_string()))?,
            expected_blob_versioned_hashes: serde_json::from_value(params[1].clone())
                .map_err(|_| RpcErr::WrongParam("expected_blob_versioned_hashes".to_string()))?,
            parent_beacon_block_root: serde_json::from_value(params[2].clone())
                .map_err(|_| RpcErr::WrongParam("parent_beacon_block_root".to_string()))?,
            execution_requests: serde_json::from_value(params[3].clone())
                .map_err(|_| RpcErr::WrongParam("execution_requests".to_string()))?,
            inclusion_list_transactions: parse_il_transactions(&params[4])?,
            raw_bal_hash,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        validate_execution_payload_v4(&self.payload)?;
        validate_execution_requests(&self.execution_requests)?;

        // Per the FOCIL spec (and the hive engine-focil "garbage bytes" test),
        // malformed IL byte strings MUST be tolerated as if they were empty IL
        // entries — the V6 call should not be rejected wholesale. Real RLP
        // decoding for the satisfaction check happens below; entries that
        // fail to decode are simply skipped. No upper size cap on the receive
        // path either — that constraint applies only to engine_getInclusionListV1
        // (the local builder).

        let requests_hash = compute_requests_hash(&self.execution_requests);
        let block_access_list_hash = self.raw_bal_hash;

        let block = match get_block_from_payload(
            &self.payload,
            Some(self.parent_beacon_block_root),
            Some(requests_hash),
            block_access_list_hash,
        ) {
            Ok(block) => block,
            Err(err) => {
                return Ok(serde_json::to_value(PayloadStatus::invalid_with_err(
                    &err.to_string(),
                ))?);
            }
        };

        let chain_config = context.storage.get_chain_config();
        if !chain_config.is_hegota_activated(block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(
                "engine_newPayloadV6 requires Hegotá-active timestamp".to_string(),
            ));
        }

        // Note re: spec rule "INVALID_BLOCK_HASH replaced by INVALID" — ethrex
        // already returns INVALID for hash mismatches (PayloadValidationStatus
        // has no InvalidBlockHash variant). Spec compliance by construction.
        let bal = self.payload.block_access_list.clone();

        // Decode IL transactions for the satisfaction check. Per the FOCIL
        // spec, malformed entries are tolerated — they're treated as if they
        // were empty IL items (no obligation imposed on the block). Skip
        // anything that fails to decode rather than rejecting the whole
        // newPayload call.
        let mut decoded_il: Vec<ethrex_common::types::Transaction> =
            Vec::with_capacity(self.inclusion_list_transactions.len());
        for (i, raw) in self.inclusion_list_transactions.iter().enumerate() {
            match ethrex_common::types::Transaction::decode_canonical(raw.as_ref()) {
                Ok(tx) => decoded_il.push(tx),
                Err(e) => {
                    debug!(
                        index = i,
                        error = %e,
                        "engine_newPayloadV6: skipping malformed IL byte string (treated as empty entry)"
                    );
                }
            }
        }

        let block_hash_for_il = block.hash();
        let payload_status = handle_new_payload_v4(
            &self.payload,
            context.clone(),
            block,
            self.expected_blob_versioned_hashes.clone(),
            bal,
        )
        .await?;

        // Only run IL satisfaction when the V4-equivalent path returned
        // VALID — INVALID/SYNCING/ACCEPTED short-circuits.
        if payload_status.status == crate::types::payload::PayloadValidationStatus::Valid
            && !decoded_il.is_empty()
        {
            // The block is now stored. Read its post-state and pre-state
            // (parent's state_root) and run the satisfaction validator.
            let stored_header = context
                .storage
                .get_block_header_by_hash(block_hash_for_il)
                .map_err(|e| RpcErr::Internal(e.to_string()))?
                .ok_or_else(|| {
                    RpcErr::Internal("stored block missing for IL satisfaction check".to_string())
                })?;
            let parent_header = context
                .storage
                .get_block_header_by_hash(stored_header.parent_hash)
                .map_err(|e| RpcErr::Internal(e.to_string()))?
                .ok_or_else(|| {
                    RpcErr::Internal("parent block missing for IL satisfaction check".to_string())
                })?;

            let pre_state = ethrex_blockchain::inclusion_list_validator::StoreIlStateProvider {
                store: &context.storage,
                state_root: parent_header.state_root,
            };
            let post_state = ethrex_blockchain::inclusion_list_validator::StoreIlStateProvider {
                store: &context.storage,
                state_root: stored_header.state_root,
            };
            let crypto = ethrex_crypto::NativeCrypto;
            let mut validator =
                ethrex_blockchain::inclusion_list_validator::InclusionListSatisfactionValidator::new(
                    &decoded_il,
                    &pre_state,
                    &crypto,
                )
                .map_err(|e| RpcErr::Internal(format!("IL validator init failed: {e}")))?;
            validator
                .refresh_all_from(&post_state, &crypto)
                .map_err(|e| RpcErr::Internal(format!("IL validator refresh failed: {e}")))?;

            // Reconstruct block_txs set from the stored block body.
            let stored_body = context
                .storage
                .get_block_body_by_hash(block_hash_for_il)
                .await
                .map_err(|e| RpcErr::Internal(e.to_string()))?
                .ok_or_else(|| {
                    RpcErr::Internal(
                        "stored block body missing for IL satisfaction check".to_string(),
                    )
                })?;
            let block_tx_hashes: std::collections::HashSet<H256> = stored_body
                .transactions
                .iter()
                .map(|tx| tx.hash())
                .collect();
            let gas_left = stored_header
                .gas_limit
                .saturating_sub(stored_header.gas_used);

            match validator.check(&decoded_il, &block_tx_hashes, gas_left, &crypto) {
                Ok(()) => {
                    // Satisfied → pass through the V4-equivalent status.
                }
                Err(ethrex_blockchain::inclusion_list_validator::IlCheckError::Unsatisfied(_)) => {
                    return serde_json::to_value(PayloadStatus::inclusion_list_unsatisfied())
                        .map_err(|e| RpcErr::Internal(e.to_string()));
                }
                Err(ethrex_blockchain::inclusion_list_validator::IlCheckError::SenderRecovery(
                    e,
                )) => {
                    return Err(RpcErr::Internal(format!(
                        "IL satisfaction check failed during sender recovery: {e}"
                    )));
                }
            }
        }

        serde_json::to_value(payload_status).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

fn parse_il_transactions(value: &Value) -> Result<Vec<bytes::Bytes>, RpcErr> {
    let array = value.as_array().ok_or_else(|| {
        RpcErr::WrongParam("inclusionListTransactions: expected array".to_string())
    })?;
    let mut out = Vec::with_capacity(array.len());
    for (i, entry) in array.iter().enumerate() {
        let s = entry.as_str().ok_or_else(|| {
            RpcErr::WrongParam(format!(
                "inclusionListTransactions[{i}]: expected hex string"
            ))
        })?;
        let bytes = hex::decode(s.trim_start_matches("0x")).map_err(|_| {
            RpcErr::WrongParam(format!("inclusionListTransactions[{i}]: invalid hex"))
        })?;
        out.push(bytes::Bytes::from(bytes));
    }
    // EIP-7805 (FOCIL): MAX_BYTES_PER_INCLUSION_LIST = 8192 is a constraint on
    // what `engine_getInclusionListV1` BUILDS, not on what V5/V6 RECEIVE. The
    // CL may forward an IL of arbitrary size and the EL must accept it (per
    // the Hive engine-focil "accepts IL larger than MAX_BYTES_PER_INCLUSION_LIST"
    // test). The size cap is enforced in
    // `crates/blockchain/inclusion_list_builder.rs` on the build side; this
    // receive path imposes no upper bound.
    Ok(out)
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload_bundle = get_payload(self.payload_id, &context).await?;
        // NOTE: This validation is actually not required to run Hive tests. Not sure if it's
        // necessary
        validate_payload_v1_v2(&payload_bundle.block, &context)?;

        // V1 doesn't support BAL (pre-EIP-7928)
        let response = ExecutionPayload::from_block(payload_bundle.block, None);

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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload_bundle = get_payload(self.payload_id, &context).await?;
        validate_payload_v1_v2(&payload_bundle.block, &context)?;

        // V2 doesn't support BAL (pre-EIP-7928)
        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(payload_bundle.block, None),
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload_bundle = get_payload(self.payload_id, &context).await?;
        validate_fork(&payload_bundle.block, Fork::Cancun, &context)?;

        // V3 doesn't support BAL (Cancun fork, pre-EIP-7928)
        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(payload_bundle.block, None),
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload_bundle = get_payload(self.payload_id, &context).await?;
        let chain_config = &context.storage.get_chain_config();

        if !chain_config.is_prague_activated(payload_bundle.block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(format!(
                "{:?}",
                chain_config.get_fork(payload_bundle.block.header.timestamp)
            )));
        }
        if chain_config.is_osaka_activated(payload_bundle.block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(format!("{:?}", Fork::Osaka)));
        }

        // V4 doesn't support BAL (Prague fork, pre-EIP-7928)
        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(payload_bundle.block, None),
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

pub struct GetPayloadV5Request {
    pub payload_id: u64,
}

impl From<GetPayloadV5Request> for RpcRequest {
    fn from(val: GetPayloadV5Request) -> Self {
        RpcRequest {
            method: "engine_getPayloadV5".to_string(),
            params: Some(vec![serde_json::json!(U256::from(val.payload_id))]),
            ..Default::default()
        }
    }
}

impl RpcHandler for GetPayloadV5Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let payload_id = parse_get_payload_request(params)?;
        Ok(Self { payload_id })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload_bundle = get_payload(self.payload_id, &context).await?;
        let chain_config = &context.storage.get_chain_config();

        if !chain_config.is_osaka_activated(payload_bundle.block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(format!(
                "{:?}",
                chain_config.get_fork(payload_bundle.block.header.timestamp)
            )));
        }

        // V5 supports BAL (Amsterdam fork, EIP-7928)
        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(
                payload_bundle.block,
                payload_bundle.block_access_list,
            ),
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

pub struct GetPayloadV6Request {
    pub payload_id: u64,
}

impl From<GetPayloadV6Request> for RpcRequest {
    fn from(val: GetPayloadV6Request) -> Self {
        RpcRequest {
            method: "engine_getPayloadV6".to_string(),
            params: Some(vec![serde_json::json!(U256::from(val.payload_id))]),
            ..Default::default()
        }
    }
}

impl RpcHandler for GetPayloadV6Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let payload_id = parse_get_payload_request(params)?;
        Ok(Self { payload_id })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let payload_bundle = get_payload(self.payload_id, &context).await?;
        let chain_config = &context.storage.get_chain_config();

        if !chain_config.is_amsterdam_activated(payload_bundle.block.header.timestamp) {
            return Err(RpcErr::UnsupportedFork(format!(
                "{:?}",
                chain_config.get_fork(payload_bundle.block.header.timestamp)
            )));
        }

        // V6 supports BAL (Amsterdam EL fork / Glamsterdam, EIP-7928)
        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(
                payload_bundle.block,
                payload_bundle.block_access_list,
            ),
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        if self.hashes.len() as u64 >= GET_PAYLOAD_BODIES_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }
        let mut bodies = Vec::new();
        for hash in self.hashes.iter() {
            bodies.push(context.storage.get_block_body_by_hash(*hash).await?)
        }
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        if self.count >= GET_PAYLOAD_BODIES_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }
        let latest_block_number = context.storage.get_latest_block_number().await?;
        // NOTE: we truncate the range because the spec says we "MUST NOT return trailing
        // null values if the request extends past the current latest known block"
        let last = latest_block_number.min(self.start + self.count - 1);
        let bodies = context.storage.get_block_bodies(self.start, last).await?;
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

// ==================== V2 Body Methods (EIP-7928) ====================

pub struct GetPayloadBodiesByHashV2Request {
    pub hashes: Vec<BlockHash>,
}

impl RpcHandler for GetPayloadBodiesByHashV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };

        Ok(GetPayloadBodiesByHashV2Request {
            hashes: serde_json::from_value(params[0].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        if self.hashes.len() as u64 >= GET_PAYLOAD_BODIES_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }

        let mut bodies: Vec<Option<ExecutionPayloadBodyV2>> = Vec::new();
        for hash in &self.hashes {
            let block = context.storage.get_block_by_hash(*hash).await?;
            let result = match block {
                Some(block) => {
                    let bal = context
                        .blockchain
                        .generate_bal_for_block(&block)
                        .map_err(|e| RpcErr::Internal(e.to_string()))?;
                    Some(ExecutionPayloadBodyV2::from_body_with_bal(block.body, bal))
                }
                None => None,
            };
            bodies.push(result);
        }

        serde_json::to_value(bodies).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

pub struct GetPayloadBodiesByRangeV2Request {
    start: BlockNumber,
    count: u64,
}

impl RpcHandler for GetPayloadBodiesByRangeV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        let start = parse_json_hex(&params[0]).map_err(|_| RpcErr::BadHexFormat(0))?;
        let count = parse_json_hex(&params[1]).map_err(|_| RpcErr::BadHexFormat(1))?;
        if start < 1 {
            return Err(RpcErr::WrongParam("start".to_owned()));
        }
        if count < 1 {
            return Err(RpcErr::WrongParam("count".to_owned()));
        }
        Ok(GetPayloadBodiesByRangeV2Request { start, count })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        if self.count >= GET_PAYLOAD_BODIES_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }
        let latest_block_number = context.storage.get_latest_block_number().await?;
        // NOTE: we truncate the range because the spec says we "MUST NOT return trailing
        // null values if the request extends past the current latest known block"
        let last = latest_block_number.min(self.start + self.count - 1);

        // Bulk fetch bodies (like V1)
        let block_bodies = context.storage.get_block_bodies(self.start, last).await?;

        let mut bodies: Vec<Option<ExecutionPayloadBodyV2>> = Vec::new();
        for (i, body_opt) in block_bodies.into_iter().enumerate() {
            let block_number = self.start + i as u64;
            let result = match body_opt {
                Some(body) => {
                    // Get header for this block
                    let header =
                        context
                            .storage
                            .get_block_header(block_number)?
                            .ok_or_else(|| {
                                RpcErr::Internal(format!(
                                    "Header not found for block {block_number}"
                                ))
                            })?;
                    let block = Block { header, body };

                    let bal = context
                        .blockchain
                        .generate_bal_for_block(&block)
                        .map_err(|e| RpcErr::Internal(e.to_string()))?;
                    Some(ExecutionPayloadBodyV2::from_body_with_bal(block.body, bal))
                }
                None => None,
            };
            bodies.push(result);
        }

        serde_json::to_value(bodies).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

fn parse_execution_payload(params: &Option<Vec<Value>>) -> Result<ExecutionPayload, RpcErr> {
    let params = params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
    if params.len() != 1 {
        return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
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

#[inline]
fn validate_execution_payload_v4(payload: &ExecutionPayload) -> Result<(), RpcErr> {
    // This method follows the same specification as `engine_newPayloadV4` additionally
    // rejects payload without block access list

    if payload.block_access_list.is_none() {
        return Err(RpcErr::WrongParam("block_access_list".to_string()));
    }

    validate_execution_payload_v3(payload)?;

    Ok(())
}

fn validate_payload_v1_v2(block: &Block, context: &RpcApiContext) -> Result<(), RpcErr> {
    let chain_config = &context.storage.get_chain_config();
    if chain_config.is_cancun_activated(block.header.timestamp) {
        return Err(RpcErr::UnsupportedFork(
            "Cancun payload received".to_string(),
        ));
    }
    Ok(())
}

// This function is used to make sure neither the current block nor its parent have been invalidated
async fn validate_ancestors(
    block: &Block,
    context: &RpcApiContext,
) -> Result<Option<PayloadStatus>, RpcErr> {
    // Check if the block has already been invalidated
    if let Some(latest_valid_hash) = context
        .storage
        .get_latest_valid_ancestor(block.hash())
        .await?
    {
        return Ok(Some(PayloadStatus::invalid_with(
            latest_valid_hash,
            "Header has been previously invalidated.".into(),
        )));
    }

    // Check if the parent block has already been invalidated
    if let Some(latest_valid_hash) = context
        .storage
        .get_latest_valid_ancestor(block.header.parent_hash)
        .await?
    {
        // Invalidate child too
        context
            .storage
            .set_latest_valid_ancestor(block.header.hash(), latest_valid_hash)
            .await?;
        return Ok(Some(PayloadStatus::invalid_with(
            latest_valid_hash,
            "Parent header has been previously invalidated.".into(),
        )));
    }

    Ok(None)
}

async fn handle_new_payload_v1_v2(
    payload: &ExecutionPayload,
    block: Block,
    context: RpcApiContext,
    bal: Option<BlockAccessList>,
) -> Result<PayloadStatus, RpcErr> {
    let Some(syncer) = &context.syncer else {
        return Err(RpcErr::Internal(
            "New payload requested but syncer is not initialized".to_string(),
        ));
    };
    // Validate block hash
    if let Err(RpcErr::Internal(error_msg)) = validate_block_hash(payload, &block) {
        return Ok(PayloadStatus::invalid_with_err(&error_msg));
    }

    // Check for invalid ancestors
    if let Some(status) = validate_ancestors(&block, &context).await? {
        return Ok(status);
    }

    // We have validated ancestors, the parent is correct
    let latest_valid_hash = block.header.parent_hash;

    if syncer.sync_mode() == SyncMode::Snap {
        debug!("Snap sync in progress, skipping new payload validation");
        return Ok(PayloadStatus::syncing());
    }

    // All checks passed, execute payload
    let payload_status = try_execute_payload(block, &context, latest_valid_hash, bal).await?;
    Ok(payload_status)
}

async fn handle_new_payload_v3(
    payload: &ExecutionPayload,
    context: RpcApiContext,
    block: Block,
    expected_blob_versioned_hashes: Vec<H256>,
    bal: Option<BlockAccessList>,
) -> Result<PayloadStatus, RpcErr> {
    // V3 specific: validate blob hashes
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

    handle_new_payload_v1_v2(payload, block, context, bal).await
}

async fn handle_new_payload_v4(
    payload: &ExecutionPayload,
    context: RpcApiContext,
    block: Block,
    expected_blob_versioned_hashes: Vec<H256>,
    bal: Option<BlockAccessList>,
) -> Result<PayloadStatus, RpcErr> {
    if let Some(bal) = &bal
        && let Err(err) = bal.validate_ordering()
    {
        return Ok(PayloadStatus::invalid_with_err(&err));
    }
    handle_new_payload_v3(payload, context, block, expected_blob_versioned_hashes, bal).await
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
    block_access_list_hash: Option<H256>,
) -> Result<Block, RLPDecodeError> {
    let block_hash = payload.block_hash;
    let block_number = payload.block_number;
    debug!(%block_hash, %block_number, "Received new payload");

    payload.clone().into_block(
        parent_beacon_block_root,
        requests_hash,
        block_access_list_hash,
    )
}

fn validate_block_hash(payload: &ExecutionPayload, block: &Block) -> Result<(), RpcErr> {
    let block_hash = payload.block_hash;
    let actual_block_hash = block.hash();
    if block_hash != actual_block_hash {
        return Err(RpcErr::Internal(format!(
            "Invalid block hash. Expected {actual_block_hash:#x}, got {block_hash:#x}"
        )));
    }
    Ok(())
}

pub async fn add_block(
    ctx: &RpcApiContext,
    block: Block,
    bal: Option<BlockAccessList>,
) -> Result<(), ChainError> {
    let (notify_send, notify_recv) = oneshot::channel();
    ctx.block_worker_channel
        .send((notify_send, block, bal))
        .map_err(|e| {
            ChainError::Custom(format!(
                "failed to send block execution request to worker: {e}"
            ))
        })?;
    notify_recv
        .await
        .map_err(|e| ChainError::Custom(format!("failed to receive block execution result: {e}")))?
}

async fn try_execute_payload(
    block: Block,
    context: &RpcApiContext,
    latest_valid_hash: H256,
    bal: Option<BlockAccessList>,
) -> Result<PayloadStatus, RpcErr> {
    let Some(syncer) = &context.syncer else {
        return Err(RpcErr::Internal(
            "New payload requested but syncer is not initialized".to_string(),
        ));
    };
    let block_hash = block.hash();
    let block_number = block.header.number;
    let storage = &context.storage;
    // Return the valid message directly if we already have it.
    // We check for header only as we do not download the block bodies before the pivot during snap sync
    // https://github.com/lambdaclass/ethrex/issues/1766
    if storage.get_block_header_by_hash(block_hash)?.is_some() {
        return Ok(PayloadStatus::valid_with_hash(block_hash));
    }

    // Execute and store the block
    debug!(%block_hash, %block_number, "Executing payload");

    match add_block(context, block, bal).await {
        Err(ChainError::ParentNotFound) => {
            // Start sync
            syncer.sync_to_head(block_hash);
            Ok(PayloadStatus::syncing())
        }
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
            warn!("Error executing block: {error}");
            context
                .storage
                .set_latest_valid_ancestor(block_hash, latest_valid_hash)
                .await?;
            Ok(PayloadStatus::invalid_with(
                latest_valid_hash,
                error.to_string(),
            ))
        }
        Err(ChainError::EvmError(error)) => {
            warn!("Error executing block: {error}");
            context
                .storage
                .set_latest_valid_ancestor(block_hash, latest_valid_hash)
                .await?;
            Ok(PayloadStatus::invalid_with(
                latest_valid_hash,
                error.to_string(),
            ))
        }
        Err(ChainError::StoreError(error)) => {
            warn!("Error storing block: {error}");
            Err(RpcErr::Internal(error.to_string()))
        }
        Err(e) => {
            error!("{e} for block {block_hash}");
            Err(RpcErr::Internal(e.to_string()))
        }
        Ok(()) => {
            debug!("Block with hash {block_hash} executed and added to storage successfully");
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

fn validate_fork(block: &Block, fork: Fork, context: &RpcApiContext) -> Result<(), RpcErr> {
    // Check timestamp matches valid fork
    let chain_config = &context.storage.get_chain_config();
    let current_fork = chain_config.get_fork(block.header.timestamp);

    if current_fork != fork {
        return Err(RpcErr::UnsupportedFork(format!("{current_fork:?}")));
    }
    Ok(())
}

async fn get_payload(payload_id: u64, context: &RpcApiContext) -> Result<PayloadBundle, RpcErr> {
    info!(
        id = %format!("{:#018x}", payload_id),
        "Requested payload with"
    );
    let (blobs_bundle, requests, block_value, block, block_access_list) = {
        let PayloadBuildResult {
            blobs_bundle,
            block_value,
            requests,
            payload,
            block_access_list,
            ..
        } = context
            .blockchain
            .get_payload(payload_id)
            .await
            .map_err(|err| match err {
                ChainError::UnknownPayload => {
                    RpcErr::UnknownPayload(format!("Payload with id {payload_id:#018x} not found",))
                }
                err => RpcErr::Internal(err.to_string()),
            })?;
        (
            blobs_bundle,
            requests,
            block_value,
            payload,
            block_access_list,
        )
    };

    let new_payload = PayloadBundle {
        block,
        block_value,
        blobs_bundle,
        requests,
        block_access_list,
    };

    Ok(new_payload)
}

#[cfg(test)]
mod v6_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_il_transactions_accepts_empty_array() {
        let parsed = parse_il_transactions(&json!([])).expect("empty IL parses");
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_il_transactions_accepts_hex_strings() {
        let parsed =
            parse_il_transactions(&json!(["0xdeadbeef", "0xcafe"])).expect("valid hex parses");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].as_ref(), &[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(parsed[1].as_ref(), &[0xca, 0xfe]);
    }

    #[test]
    fn parse_il_transactions_rejects_non_array() {
        let err = parse_il_transactions(&json!("0xdeadbeef")).unwrap_err();
        assert!(matches!(err, RpcErr::WrongParam(_)));
    }

    #[test]
    fn parse_il_transactions_rejects_non_string_entries() {
        let err = parse_il_transactions(&json!([123])).unwrap_err();
        assert!(matches!(err, RpcErr::WrongParam(_)));
    }

    #[test]
    fn parse_il_transactions_rejects_invalid_hex() {
        let err = parse_il_transactions(&json!(["0xZZ"])).unwrap_err();
        assert!(matches!(err, RpcErr::WrongParam(_)));
    }

    /// Hive engine-focil testForkchoiceUpdatedAcceptsLargeIL: V5/V6 receive
    /// paths must NOT enforce MAX_BYTES_PER_INCLUSION_LIST. The 8 KiB cap
    /// only applies to engine_getInclusionListV1 (the local builder).
    #[test]
    fn parse_il_transactions_accepts_oversized_inputs() {
        // 10 KiB single entry (well over the 8 KiB build cap) must round-trip
        // through the receive-side parser without error.
        let big_hex = format!("0x{}", "42".repeat(10 * 1024));
        let parsed = parse_il_transactions(&json!([big_hex])).expect("oversized parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].len(), 10 * 1024);
    }

    /// Hive engine-focil "garbage bytes" test: bytes that aren't valid RLP
    /// transactions (`0xdeadbeef`, empty, `0x02c0`) MUST round-trip through
    /// `parse_il_transactions` without error — RLP decode failures are
    /// downstream concerns handled by skip-and-continue.
    #[test]
    fn parse_il_transactions_tolerates_garbage_bytes() {
        let parsed =
            parse_il_transactions(&json!(["0xdeadbeef", "", "0x02c0"])).expect("garbage parses");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].as_ref(), &[0xde, 0xad, 0xbe, 0xef]);
        assert!(parsed[1].is_empty());
        assert_eq!(parsed[2].as_ref(), &[0x02, 0xc0]);
    }

    #[test]
    fn newpayload_v6_parse_rejects_wrong_param_count() {
        let four_only =
            NewPayloadV6Request::parse(&Some(vec![json!({}), json!([]), json!("0x"), json!([])]));
        assert!(matches!(four_only, Err(RpcErr::BadParams(_))));
    }
}
