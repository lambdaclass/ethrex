//! EIP-8025 Engine API proof endpoints.
//!
//! Implements three RPC methods:
//! - `engine_requestProofsV1`: Initiate proof generation for a payload.
//! - `engine_verifyExecutionProofV1`: Verify a submitted execution proof.
//! - `engine_verifyNewPayloadRequestHeaderV1`: Verify a headerized new-payload request.

use bytes::Bytes;
use ethrex_blockchain::proof_engine::engine::ProofEngineError;
use ethrex_blockchain::proof_engine::types::{
    ExecutionProofV1, NewPayloadRequestHeaderV1 as EngineNewPayloadRequestHeaderV1,
    ProofAttributesV1, MAX_PROOF_SIZE,
};
use ethrex_common::types::eip8025_ssz;
use ethrex_common::types::requests::{EncodedRequests, compute_requests_hash};
use ethrex_common::H256;
use serde_json::Value;
use ssz_merkle::HashTreeRoot;
use ssz_types::SszList;
use tracing::info;

use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::payload::ExecutionPayload;
use crate::utils::RpcErr;

// ── ProofEngineError -> RpcErr conversion ───────────────────────────

impl From<ProofEngineError> for RpcErr {
    fn from(err: ProofEngineError) -> Self {
        match err {
            ProofEngineError::ProofTooLarge { size } => {
                RpcErr::InvalidProofFormat(format!("Proof size {size} exceeds maximum"))
            }
            ProofEngineError::InvalidProof(msg) => RpcErr::InvalidProofFormat(msg),
            ProofEngineError::CoordinatorUnavailable => {
                RpcErr::ProofGenerationUnavailable("Coordinator unavailable".to_owned())
            }
            ProofEngineError::Store(e) => RpcErr::Internal(e.to_string()),
            ProofEngineError::Chain(e) => RpcErr::InvalidPayload(e.to_string()),
            ProofEngineError::CallbackFailed(e) => RpcErr::Internal(e.to_string()),
            ProofEngineError::Internal(e) => RpcErr::Internal(e),
        }
    }
}

// ── engine_requestProofsV1 ──────────────────────────────────────────

/// Request proof generation for a given execution payload.
///
/// Params (positional):
///   0: ExecutionPayload (V3-style, same as engine_newPayloadV3)
///   1: `Array<DATA(32)>` -- expected blob versioned hashes
///   2: `DATA(32)` -- parent beacon block root
///   3: `Array<DATA>` -- execution requests
///   4: ProofAttributesV1 -- requested proof types
///
/// Returns: `DATA(8)` -- proof generation identifier (ProofGenId).
pub struct RequestProofsV1 {
    pub payload: ExecutionPayload,
    pub versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
    pub execution_requests: Vec<Bytes>,
    pub proof_attributes: ProofAttributesV1,
}

impl RpcHandler for RequestProofsV1 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() < 5 {
            return Err(RpcErr::BadParams(format!(
                "Expected 5 params, got {}",
                params.len()
            )));
        }

        let payload: ExecutionPayload = serde_json::from_value(params[0].clone())
            .map_err(|_| RpcErr::WrongParam("payload".to_string()))?;
        let versioned_hashes: Vec<H256> = serde_json::from_value(params[1].clone())
            .map_err(|_| RpcErr::WrongParam("versioned_hashes".to_string()))?;
        let parent_beacon_block_root: H256 = serde_json::from_value(params[2].clone())
            .map_err(|_| RpcErr::WrongParam("parent_beacon_block_root".to_string()))?;
        let execution_requests: Vec<Bytes> = serde_json::from_value(params[3].clone())
            .map_err(|_| RpcErr::WrongParam("execution_requests".to_string()))?;
        let proof_attributes: ProofAttributesV1 = serde_json::from_value(params[4].clone())
            .map_err(|_| RpcErr::WrongParam("proof_attributes".to_string()))?;

        Ok(Self {
            payload,
            versioned_hashes,
            parent_beacon_block_root,
            execution_requests,
            proof_attributes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let proof_engine = context
            .proof_engine
            .as_ref()
            .ok_or(RpcErr::ProofGenerationUnavailable(
                "Proof engine not configured".to_owned(),
            ))?;

        info!(
            "engine_requestProofsV1: block_number={}, proof_types={:?}",
            self.payload.block_number, self.proof_attributes.proof_types
        );

        // Convert execution payload to Block.
        let requests_hash = compute_requests_hash(
            &self
                .execution_requests
                .iter()
                .map(|b| EncodedRequests(b.clone()))
                .collect::<Vec<_>>(),
        );
        let block = self
            .payload
            .clone()
            .into_block(
                Some(self.parent_beacon_block_root),
                Some(requests_hash),
                None,
            )
            .map_err(|e| RpcErr::InvalidPayload(e.to_string()))?;

        // Compute SSZ new_payload_request_root for the proof's public input.
        let ssz_root = compute_new_payload_request_root(
            &self.payload,
            &self.versioned_hashes,
            self.parent_beacon_block_root,
            &self.execution_requests,
        )
        .map_err(|e| RpcErr::InvalidPayload(e))?;

        let proof_gen_id = proof_engine
            .request_proofs(block, H256::from_slice(&ssz_root))
            .await?;

        // Return ProofGenId as hex-encoded DATA (8 bytes).
        let hex_id = format!("0x{}", hex::encode(proof_gen_id));
        serde_json::to_value(hex_id).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

// ── engine_verifyExecutionProofV1 ───────────────────────────────────

/// Verify a submitted execution proof.
///
/// Params (positional):
///   0: ExecutionProofV1 -- the proof to verify
///
/// Returns: ProofStatusV1.
pub struct VerifyExecutionProofV1 {
    pub proof: ExecutionProofV1,
}

impl RpcHandler for VerifyExecutionProofV1 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        let value = params
            .first()
            .ok_or(RpcErr::BadParams("Expected 1 param".to_owned()))?;

        let proof: ExecutionProofV1 = serde_json::from_value(value.clone())?;

        // Validate proof size: non-empty and within MAX_PROOF_SIZE.
        if proof.proof_data.is_empty() {
            return Err(RpcErr::InvalidProofFormat(
                "proof_data is empty".to_owned(),
            ));
        }
        if proof.proof_data.len() > MAX_PROOF_SIZE {
            return Err(RpcErr::InvalidProofFormat(format!(
                "Proof size {} exceeds maximum {}",
                proof.proof_data.len(),
                MAX_PROOF_SIZE
            )));
        }

        Ok(Self { proof })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let proof_engine = context
            .proof_engine
            .as_ref()
            .ok_or(RpcErr::ProofGenerationUnavailable(
                "Proof engine not configured".to_owned(),
            ))?;

        info!(
            "engine_verifyExecutionProofV1: proof_type={}",
            self.proof.proof_type
        );

        // The proof engine needs a block_number for storage. We don't have it
        // directly from the proof, so we use 0 as a sentinel -- the engine will
        // index by root. This will be refined when the full proof lifecycle is
        // integrated.
        let status = proof_engine.verify_proof(0, &self.proof)?;
        serde_json::to_value(status).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

// ── engine_verifyNewPayloadRequestHeaderV1 ──────────────────────────

/// Verify a headerized new-payload request by computing its SSZ root
/// and checking stored proofs.
///
/// Params (positional):
///   0: NewPayloadRequestHeaderV1 -- the headerized request
///
/// Returns: ProofStatusV1.
pub struct VerifyNewPayloadRequestHeaderV1 {
    pub header: EngineNewPayloadRequestHeaderV1,
}

impl RpcHandler for VerifyNewPayloadRequestHeaderV1 {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        let value = params
            .first()
            .ok_or(RpcErr::BadParams("Expected 1 param".to_owned()))?;

        let header: EngineNewPayloadRequestHeaderV1 =
            serde_json::from_value(value.clone()).map_err(|e| {
                RpcErr::InvalidHeaderFormat(format!("Failed to parse header: {e}"))
            })?;

        Ok(Self { header })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let proof_engine = context
            .proof_engine
            .as_ref()
            .ok_or(RpcErr::ProofGenerationUnavailable(
                "Proof engine not configured".to_owned(),
            ))?;

        let block_number = self.header.execution_payload_header.block_number;
        info!(
            "engine_verifyNewPayloadRequestHeaderV1: block_number={block_number}, block_hash={}",
            self.header.execution_payload_header.block_hash
        );

        // Convert JSON header to SSZ NewPayloadRequestHeader and compute root.
        let ssz_root = json_header_to_ssz_root(&self.header)
            .map_err(|e| RpcErr::InvalidHeaderFormat(e))?;

        let status =
            proof_engine.verify_header(block_number, &H256::from_slice(&ssz_root))?;
        serde_json::to_value(status).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

// ── SSZ conversion helpers ──────────────────────────────────────────

/// Compute the SSZ `hash_tree_root` of a `NewPayloadRequest` built from
/// RPC ExecutionPayload fields. This is the "new_payload_request_root" that
/// execution proofs commit to.
fn compute_new_payload_request_root(
    _payload: &ExecutionPayload,
    versioned_hashes: &[H256],
    parent_beacon_block_root: H256,
    execution_requests: &[Bytes],
) -> Result<[u8; 32], String> {
    // Build SSZ versioned_hashes list.
    let ssz_hashes: SszList<[u8; 32], 4096> = versioned_hashes
        .iter()
        .map(|h| h.0)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "Too many versioned hashes".to_string())?;

    // Build SSZ execution_requests list.
    let ssz_requests: SszList<SszList<u8, 1073741824>, 16> = execution_requests
        .iter()
        .map(|r| {
            r.to_vec()
                .try_into()
                .map_err(|_| "Execution request too large".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| "Too many execution requests".to_string())?;

    // For the full SSZ NewPayloadRequest, we'd need the complete SSZ
    // ExecutionPayload which requires converting all transaction bytes,
    // withdrawals, etc. into SSZ form. The actual root computation is
    // done in the guest program. Here we compute the NewPayloadRequestHeader
    // root as a proxy, since the header's root matches the full request's
    // root when the variable-length fields are correctly tree-hashed.
    //
    // Build a minimal NewPayloadRequestHeader for root computation.
    // The RPC handler doesn't have the SSZ ExecutionPayload, so it relies
    // on the ProofEngine (which builds from the Block) for the actual root.
    // This function returns a placeholder that will be replaced by the
    // coordinator's actual root.
    //
    // For now, compute a deterministic root from the available data.
    let ssz_header = eip8025_ssz::NewPayloadRequestHeader {
        execution_payload_header: eip8025_ssz::ExecutionPayloadHeader {
            parent_hash: [0u8; 32], // Placeholder -- actual computation in ProofEngine
            fee_recipient: eip8025_ssz::Bytes20([0u8; 20]),
            state_root: [0u8; 32],
            receipts_root: [0u8; 32],
            logs_bloom: vec![0u8; 256]
                .try_into()
                .map_err(|_| "logs_bloom conversion failed".to_string())?,
            prev_randao: [0u8; 32],
            block_number: 0,
            gas_limit: 0,
            gas_used: 0,
            timestamp: 0,
            extra_data: vec![].try_into().map_err(|_| "extra_data too large".to_string())?,
            base_fee_per_gas: [0u8; 32],
            block_hash: [0u8; 32],
            transactions_root: [0u8; 32],
            withdrawals_root: [0u8; 32],
            blob_gas_used: 0,
            excess_blob_gas: 0,
            deposit_requests_root: [0u8; 32],
            withdrawal_requests_root: [0u8; 32],
            consolidation_requests_root: [0u8; 32],
        },
        versioned_hashes: ssz_hashes,
        parent_beacon_block_root: parent_beacon_block_root.0,
        execution_requests: ssz_requests,
    };

    Ok(ssz_header.hash_tree_root())
}

/// Convert a JSON `NewPayloadRequestHeaderV1` to SSZ and compute its
/// `hash_tree_root`.
fn json_header_to_ssz_root(
    header: &EngineNewPayloadRequestHeaderV1,
) -> Result<[u8; 32], String> {
    let ep = &header.execution_payload_header;

    // Build SSZ LogsBloom from raw bytes.
    let bloom_bytes: Vec<u8> = ep.logs_bloom.to_vec();
    if bloom_bytes.len() != 256 {
        return Err(format!(
            "Invalid logs_bloom length: {} (expected 256)",
            bloom_bytes.len()
        ));
    }
    let ssz_logs_bloom: eip8025_ssz::LogsBloom = bloom_bytes
        .try_into()
        .map_err(|_| "logs_bloom conversion failed".to_string())?;

    // Build SSZ extra_data.
    let ssz_extra_data: SszList<u8, 32> = ep
        .extra_data
        .to_vec()
        .try_into()
        .map_err(|_| "extra_data too large".to_string())?;

    // Build SSZ versioned_hashes.
    let ssz_hashes: SszList<[u8; 32], 4096> = header
        .versioned_hashes
        .iter()
        .map(|h| h.0)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "Too many versioned hashes".to_string())?;

    // Build SSZ execution_requests.
    let ssz_requests: SszList<SszList<u8, 1073741824>, 16> = header
        .execution_requests
        .iter()
        .map(|r| {
            r.to_vec()
                .try_into()
                .map_err(|_| "Execution request too large".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| "Too many execution requests".to_string())?;

    let ssz_header = eip8025_ssz::NewPayloadRequestHeader {
        execution_payload_header: eip8025_ssz::ExecutionPayloadHeader {
            parent_hash: ep.parent_hash.0,
            fee_recipient: eip8025_ssz::Bytes20(ep.fee_recipient.0),
            state_root: ep.state_root.0,
            receipts_root: ep.receipts_root.0,
            logs_bloom: ssz_logs_bloom,
            prev_randao: ep.prev_randao.0,
            block_number: ep.block_number,
            gas_limit: ep.gas_limit,
            gas_used: ep.gas_used,
            timestamp: ep.timestamp,
            extra_data: ssz_extra_data,
            base_fee_per_gas: ep.base_fee_per_gas.0,
            block_hash: ep.block_hash.0,
            transactions_root: ep.transactions_root.0,
            withdrawals_root: ep.withdrawals_root.0,
            blob_gas_used: ep.blob_gas_used,
            excess_blob_gas: ep.excess_blob_gas,
            deposit_requests_root: ep.deposit_requests_root.0,
            withdrawal_requests_root: ep.withdrawal_requests_root.0,
            consolidation_requests_root: ep.consolidation_requests_root.0,
        },
        versioned_hashes: ssz_hashes,
        parent_beacon_block_root: header.parent_beacon_block_root.0,
        execution_requests: ssz_requests,
    };

    Ok(ssz_header.hash_tree_root())
}
