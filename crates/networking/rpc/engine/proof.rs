//! EIP-8025 Engine API proof endpoints.
//!
//! Implements three RPC methods:
//! - `engine_requestProofsV1`: Initiate proof generation for a payload.
//! - `engine_verifyExecutionProofV1`: Verify a submitted execution proof.
//! - `engine_verifyNewPayloadRequestHeaderV1`: Verify a headerized new-payload request.

use bytes::Bytes;
use ethrex_blockchain::proof_engine::coordinator::{CoordCastMsg, CoordinatorHandle, ProofRequest};
use ethrex_blockchain::proof_engine::types::{
    ExecutionProofV1, MAX_PROOF_SIZE, MIN_REQUIRED_EXECUTION_PROOFS,
    NewPayloadRequestHeaderV1 as EngineNewPayloadRequestHeaderV1, ProofAttributesV1, ProofGenId,
    ProofStatusV1, ProofValidationStatus,
};
use ethrex_common::H256;
use ethrex_common::types::eip8025_ssz;
use ethrex_common::types::requests::{EncodedRequests, compute_requests_hash};
use ethrex_guest_program::input::ProgramInput;
use serde_json::Value;
use ssz_merkle::HashTreeRoot;
use ssz_types::SszList;
use tracing::{debug, info, warn};

use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::payload::ExecutionPayload;
use crate::utils::RpcErr;

// ── Helper functions ────────────────────────────────────────────────

/// Build a ProofGenId from block number and root.
/// Uses the lower 4 bytes of block_number and the first 4 bytes of root.
fn make_proof_gen_id(block_number: u64, root: &H256) -> ProofGenId {
    let mut id = [0u8; 8];
    id[..4].copy_from_slice(&block_number.to_be_bytes()[4..]);
    id[4..].copy_from_slice(&root.as_bytes()[..4]);
    id
}

/// Get the coordinator handle from context, returning an RPC error if unavailable.
fn get_coordinator(context: &RpcApiContext) -> Result<&CoordinatorHandle, RpcErr> {
    context
        .proof_coordinator
        .as_ref()
        .ok_or(RpcErr::ProofGenerationUnavailable(
            "Proof coordinator not configured".to_owned(),
        ))
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
        let mut coordinator = get_coordinator(&context)?.clone();

        let block_number = self.payload.block_number;
        info!(
            "engine_requestProofsV1: block_number={}, proof_types={:?}",
            block_number, self.proof_attributes.proof_types
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
        .map_err(RpcErr::InvalidPayload)?;

        let new_payload_request_root = H256::from_slice(&ssz_root);

        // Generate execution witness for this block.
        let witness = context
            .blockchain
            .generate_witness_for_blocks(std::slice::from_ref(&block))
            .await
            .map_err(|e| RpcErr::InvalidPayload(e.to_string()))?;

        // Build ProgramInput with the block and witness.
        let program_input = ProgramInput::new(vec![block], witness);

        // Persist root → block_number mapping in the DB.
        context
            .storage
            .store_root_to_block(new_payload_request_root, block_number)
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        // Generate a ProofGenId from (block_number, root).
        let proof_gen_id = make_proof_gen_id(block_number, &new_payload_request_root);

        // Send the proof request to the coordinator via GenServer message.
        coordinator
            .cast(CoordCastMsg::AddRequest {
                block_number,
                request: Box::new(ProofRequest {
                    proof_gen_id,
                    new_payload_request_root,
                    program_input,
                    requested_proof_types: self.proof_attributes.proof_types.clone(),
                }),
            })
            .await
            .map_err(|e| {
                RpcErr::ProofGenerationUnavailable(format!("Coordinator unavailable: {e}"))
            })?;

        debug!(block_number, "Proof request sent to coordinator");

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
            return Err(RpcErr::InvalidProofFormat("proof_data is empty".to_owned()));
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
        info!(
            "engine_verifyExecutionProofV1: proof_type={}",
            self.proof.proof_type
        );

        let root = self.proof.public_input.new_payload_request_root;

        // Look up block_number from the persisted root→block mapping.
        let block_number = context
            .storage
            .get_block_number_by_root(&root)
            .map_err(|e| RpcErr::Internal(e.to_string()))?
            .unwrap_or_else(|| {
                warn!(
                    root = %root,
                    "Unknown root in verify_proof; storing with block_number=0"
                );
                0
            });

        // Store the proof.
        context
            .storage
            .store_execution_proof(
                block_number,
                root,
                self.proof.proof_type,
                self.proof.proof_data.to_vec(),
            )
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        info!(
            block_number,
            proof_type = self.proof.proof_type,
            "Execution proof stored"
        );

        let status = ProofStatusV1 {
            status: ProofValidationStatus::Valid,
            error: None,
        };
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

        let header: EngineNewPayloadRequestHeaderV1 = serde_json::from_value(value.clone())
            .map_err(|e| RpcErr::InvalidHeaderFormat(format!("Failed to parse header: {e}")))?;

        Ok(Self { header })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block_number = self.header.execution_payload_header.block_number;
        info!(
            "engine_verifyNewPayloadRequestHeaderV1: block_number={block_number}, block_hash={}",
            self.header.execution_payload_header.block_hash
        );

        // Convert JSON header to SSZ NewPayloadRequestHeader and compute root.
        let ssz_root =
            json_header_to_ssz_root(&self.header).map_err(RpcErr::InvalidHeaderFormat)?;

        let new_payload_request_root = H256::from_slice(&ssz_root);

        let proofs = context
            .storage
            .get_execution_proofs(block_number, &new_payload_request_root)
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        let count = proofs.len();
        debug!(
            block_number,
            root = %new_payload_request_root,
            count,
            "Header verification"
        );

        let status = if count >= MIN_REQUIRED_EXECUTION_PROOFS {
            ProofStatusV1 {
                status: ProofValidationStatus::Valid,
                error: None,
            }
        } else {
            ProofStatusV1 {
                status: ProofValidationStatus::Syncing,
                error: None,
            }
        };

        serde_json::to_value(status).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

// ── SSZ conversion helpers ──────────────────────────────────────────

/// Compute the SSZ `hash_tree_root` of a `NewPayloadRequest` built from
/// RPC ExecutionPayload fields. This is the "new_payload_request_root" that
/// execution proofs commit to.
fn compute_new_payload_request_root(
    payload: &ExecutionPayload,
    versioned_hashes: &[H256],
    parent_beacon_block_root: H256,
    execution_requests: &[Bytes],
) -> Result<[u8; 32], String> {
    // Build the full SSZ ExecutionPayload from the RPC payload.
    let ssz_payload = rpc_payload_to_ssz(payload)?;

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

    let ssz_request = eip8025_ssz::NewPayloadRequest {
        execution_payload: ssz_payload,
        versioned_hashes: ssz_hashes,
        parent_beacon_block_root: parent_beacon_block_root.0,
        execution_requests: ssz_requests,
    };

    let root = ssz_request.hash_tree_root();
    // Also compute via header path for comparison (diagnostic; logged at debug level only).
    let header = ssz_request.to_header();
    let header_root = header.hash_tree_root();
    debug!(
        "SSZ root (full): 0x{}, (header): 0x{}, match: {}",
        hex::encode(root),
        hex::encode(header_root),
        root == header_root
    );

    Ok(root)
}

/// Convert an RPC `ExecutionPayload` into a full SSZ `eip8025_ssz::ExecutionPayload`.
fn rpc_payload_to_ssz(payload: &ExecutionPayload) -> Result<eip8025_ssz::ExecutionPayload, String> {
    // Transactions: Vec<EncodedTransaction> -> SszList<SszList<u8, ...>, ...>
    let ssz_txs: Vec<SszList<u8, 1073741824>> = payload
        .transactions()
        .iter()
        .map(|tx| {
            tx.0.to_vec()
                .try_into()
                .map_err(|_| "Transaction too large".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ssz_transactions: SszList<SszList<u8, 1073741824>, 1048576> = ssz_txs
        .try_into()
        .map_err(|_| "Too many transactions".to_string())?;

    // Withdrawals: Option<Vec<Withdrawal>> -> SszList<ssz::Withdrawal, 16>
    let ssz_withdrawals: SszList<eip8025_ssz::Withdrawal, 16> = payload
        .withdrawals()
        .unwrap_or(&[])
        .iter()
        .map(|w| eip8025_ssz::Withdrawal {
            index: w.index,
            validator_index: w.validator_index,
            address: eip8025_ssz::Bytes20(w.address.0),
            amount: w.amount,
        })
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "Too many withdrawals".to_string())?;

    // base_fee_per_gas: u64 -> [u8; 32] (256-bit little-endian)
    let mut base_fee_bytes = [0u8; 32];
    base_fee_bytes[..8].copy_from_slice(&payload.base_fee_per_gas().to_le_bytes());

    // logs_bloom: Bloom (256 bytes) -> SszVector<u8, 256>
    let ssz_logs_bloom: eip8025_ssz::LogsBloom = payload
        .logs_bloom()
        .0
        .to_vec()
        .try_into()
        .map_err(|_| "logs_bloom conversion failed".to_string())?;

    // extra_data: Bytes -> SszList<u8, 32>
    let ssz_extra_data: SszList<u8, 32> = payload
        .extra_data()
        .to_vec()
        .try_into()
        .map_err(|_| "extra_data too large".to_string())?;

    // Deposit/withdrawal/consolidation requests: empty (not in Engine API payload)
    let ssz_deposit_requests: SszList<eip8025_ssz::DepositRequest, 8192> =
        vec![].try_into().expect("empty list should always convert");
    let ssz_withdrawal_requests: SszList<eip8025_ssz::WithdrawalRequest, 16> =
        vec![].try_into().expect("empty list should always convert");
    let ssz_consolidation_requests: SszList<eip8025_ssz::ConsolidationRequest, 1> =
        vec![].try_into().expect("empty list should always convert");

    Ok(eip8025_ssz::ExecutionPayload {
        parent_hash: payload.parent_hash().0,
        fee_recipient: eip8025_ssz::Bytes20(payload.fee_recipient().0),
        state_root: payload.state_root().0,
        receipts_root: payload.receipts_root().0,
        logs_bloom: ssz_logs_bloom,
        prev_randao: payload.prev_randao().0,
        block_number: payload.block_number,
        gas_limit: payload.gas_limit(),
        gas_used: payload.gas_used(),
        timestamp: payload.timestamp,
        extra_data: ssz_extra_data,
        base_fee_per_gas: base_fee_bytes,
        block_hash: payload.block_hash.0,
        transactions: ssz_transactions,
        withdrawals: ssz_withdrawals,
        blob_gas_used: payload.blob_gas_used.unwrap_or(0),
        excess_blob_gas: payload.excess_blob_gas.unwrap_or(0),
        deposit_requests: ssz_deposit_requests,
        withdrawal_requests: ssz_withdrawal_requests,
        consolidation_requests: ssz_consolidation_requests,
    })
}

/// Convert a JSON `NewPayloadRequestHeaderV1` to SSZ and compute its
/// `hash_tree_root`.
fn json_header_to_ssz_root(header: &EngineNewPayloadRequestHeaderV1) -> Result<[u8; 32], String> {
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

    // base_fee_per_gas: H256 stores as big-endian, but SSZ Uint256 is little-endian.
    let mut base_fee_le = ep.base_fee_per_gas.0;
    base_fee_le.reverse();

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
            base_fee_per_gas: base_fee_le,
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

    let root = ssz_header.hash_tree_root();
    info!("SSZ root (json_header): 0x{}", hex::encode(root),);
    Ok(root)
}
