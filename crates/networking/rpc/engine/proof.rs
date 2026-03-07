//! EIP-8025 Engine API proof endpoints.
//!
//! - `engine_requestProofsV1`
//! - `engine_verifyExecutionProofV1`
//! - `engine_verifyNewPayloadRequestHeaderV1`

use ethrex_blockchain::proof_engine::ProofEngine;
use ethrex_blockchain::proof_engine::ssz::{
    SszExecutionPayloadHeader, SszNewPayloadRequestHeader,
};
use ethrex_blockchain::proof_engine::types::{self as proof_types, MAX_PROOF_SIZE};
use ethrex_common::H256;
use serde_json::Value;
use ssz_rs::prelude::*;

use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::proof::{
    ExecutionProofV1, NewPayloadRequestHeaderV1, ProofAttributesV1, ProofStatusV1,
};
use crate::utils::RpcErr;

// ── engine_requestProofsV1 ──────────────────────────────────────────

pub struct RequestProofsV1 {
    pub block_number: u64,
    pub block_hash: H256,
    pub proof_attributes: ProofAttributesV1,
}

impl RpcHandler for RequestProofsV1 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or_else(|| RpcErr::BadParams("No params provided".to_string()))?;

        if params.len() < 5 {
            return Err(RpcErr::BadParams(
                "engine_requestProofsV1 requires 5 parameters".to_string(),
            ));
        }

        // Extract block_number and block_hash from executionPayload (param 0)
        let payload = &params[0];
        let block_number = payload
            .get("blockNumber")
            .and_then(|v| v.as_str())
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .ok_or_else(|| RpcErr::BadParams("invalid blockNumber in payload".to_string()))?;
        let block_hash: H256 = payload
            .get("blockHash")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or_else(|| RpcErr::BadParams("invalid blockHash in payload".to_string()))?;

        // param 4 is proofAttributes
        let proof_attributes: ProofAttributesV1 = serde_json::from_value(params[4].clone())
            .map_err(|e| RpcErr::BadParams(format!("invalid proofAttributes: {e}")))?;

        Ok(Self {
            block_number,
            block_hash,
            proof_attributes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let engine = ProofEngine::new(context.storage);

        // Convert RPC ProofAttributesV1 to internal type
        let internal_attrs = proof_types::ProofAttributesV1 {
            proof_types: self.proof_attributes.proof_types.clone(),
        };

        let proof_gen_id =
            engine.request_proofs(self.block_number, self.block_hash, &internal_attrs);
        let hex = format!("0x{}", hex::encode(proof_gen_id));
        Ok(Value::String(hex))
    }
}

// ── engine_verifyExecutionProofV1 ───────────────────────────────────

pub struct VerifyExecutionProofV1 {
    pub execution_proof: ExecutionProofV1,
}

impl RpcHandler for VerifyExecutionProofV1 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or_else(|| RpcErr::BadParams("No params provided".to_string()))?;

        let execution_proof: ExecutionProofV1 = serde_json::from_value(
            params
                .first()
                .ok_or_else(|| RpcErr::BadParams("Expected 1 param".to_string()))?
                .clone(),
        )
        .map_err(|e| RpcErr::BadParams(format!("invalid ExecutionProofV1: {e}")))?;

        // Pre-validation
        if execution_proof.proof_data.is_empty() {
            return Err(RpcErr::BadParams("proofData must not be empty".to_string()));
        }
        if execution_proof.proof_data.len() > MAX_PROOF_SIZE {
            return Err(RpcErr::BadParams(format!(
                "proofData exceeds maximum size ({} > {})",
                execution_proof.proof_data.len(),
                MAX_PROOF_SIZE
            )));
        }

        Ok(Self { execution_proof })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let engine = ProofEngine::new(context.storage.clone());

        let block_number = context
            .storage
            .get_latest_block_number()
            .await
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        // Convert RPC type to internal type
        let internal_proof = proof_types::ExecutionProofV1 {
            proof_data: self.execution_proof.proof_data.to_vec(),
            proof_type: self.execution_proof.proof_type,
            public_input: proof_types::PublicInputV1 {
                new_payload_request_root: self
                    .execution_proof
                    .public_input
                    .new_payload_request_root,
            },
        };

        let result = engine.verify_proof(block_number, &internal_proof);

        let rpc_status = ProofStatusV1 {
            status: result.status.as_str().to_string(),
            error: result.error,
        };

        serde_json::to_value(rpc_status).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

// ── engine_verifyNewPayloadRequestHeaderV1 ──────────────────────────

pub struct VerifyNewPayloadRequestHeaderV1 {
    pub header: NewPayloadRequestHeaderV1,
}

impl RpcHandler for VerifyNewPayloadRequestHeaderV1 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or_else(|| RpcErr::BadParams("No params provided".to_string()))?;

        let header: NewPayloadRequestHeaderV1 = serde_json::from_value(
            params
                .first()
                .ok_or_else(|| RpcErr::BadParams("Expected 1 param".to_string()))?
                .clone(),
        )
        .map_err(|e| RpcErr::BadParams(format!("invalid NewPayloadRequestHeaderV1: {e}")))?;

        Ok(Self { header })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let engine = ProofEngine::new(context.storage.clone());

        let mut ssz_header = rpc_header_to_ssz(&self.header)
            .map_err(|e| RpcErr::Internal(format!("failed to convert header to SSZ: {e}")))?;

        let block_number = self.header.execution_payload_header.block_number;

        let result = engine.verify_header(block_number, &mut ssz_header);

        let rpc_status = ProofStatusV1 {
            status: result.status.as_str().to_string(),
            error: result.error,
        };

        serde_json::to_value(rpc_status).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

/// Convert an RPC `NewPayloadRequestHeaderV1` to the SSZ `SszNewPayloadRequestHeader`.
fn rpc_header_to_ssz(
    header: &NewPayloadRequestHeaderV1,
) -> Result<SszNewPayloadRequestHeader, String> {
    let ep = &header.execution_payload_header;

    let mut logs_bloom = Vector::<u8, 256>::default();
    for (i, byte) in ep.logs_bloom.as_bytes().iter().enumerate() {
        if i < 256 {
            logs_bloom[i] = *byte;
        }
    }

    let mut extra_data: List<u8, 32> = List::default();
    for byte in ep.extra_data.iter() {
        extra_data.push(*byte);
    }

    let ssz_ep_header = SszExecutionPayloadHeader {
        parent_hash: h256_to_bytes32(&ep.parent_hash),
        fee_recipient: addr_to_bytes20(&ep.fee_recipient),
        state_root: h256_to_bytes32(&ep.state_root),
        receipts_root: h256_to_bytes32(&ep.receipts_root),
        logs_bloom,
        prev_randao: h256_to_bytes32(&ep.prev_randao),
        block_number: ep.block_number,
        gas_limit: ep.gas_limit,
        gas_used: ep.gas_used,
        timestamp: ep.timestamp,
        extra_data,
        base_fee_per_gas: ssz_rs::U256::default(), // TODO: proper u64→U256 conversion
        block_hash: h256_to_bytes32(&ep.block_hash),
        transactions_root: h256_to_bytes32(&ep.transactions_root),
        withdrawals_root: h256_to_bytes32(&ep.withdrawals_root),
        blob_gas_used: ep.blob_gas_used,
        excess_blob_gas: ep.excess_blob_gas,
    };

    let mut versioned_hashes: List<[u8; 32], 4096> = List::default();
    for h in &header.versioned_hashes {
        versioned_hashes.push(h256_to_bytes32(h));
    }

    let mut execution_requests: List<List<u8, 8_388_608>, 16> = List::default();
    for req_bytes in &header.execution_requests {
        let mut req_list: List<u8, 8_388_608> = List::default();
        for byte in req_bytes.iter() {
            req_list.push(*byte);
        }
        execution_requests.push(req_list);
    }

    Ok(SszNewPayloadRequestHeader {
        execution_payload_header: ssz_ep_header,
        versioned_hashes,
        parent_beacon_block_root: h256_to_bytes32(&header.parent_beacon_block_root),
        execution_requests,
    })
}

fn h256_to_bytes32(h: &H256) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(h.as_bytes());
    bytes
}

fn addr_to_bytes20(a: &ethrex_common::Address) -> [u8; 20] {
    let mut bytes = [0u8; 20];
    bytes.copy_from_slice(a.as_bytes());
    bytes
}
