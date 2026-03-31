//! EIP-8025 Engine API proof endpoints.
//!
//! Implements three RPC methods:
//! - `engine_requestProofsV1`: Initiate proof generation for a payload.
//! - `engine_verifyExecutionProofV1`: Verify a submitted execution proof.
//! - `engine_verifyNewPayloadRequestHeaderV1`: Verify a headerized new-payload request.

use bytes::Bytes;
use ethrex_blockchain::proof_coordinator::coordinator::{
    CoordinatorHandle, ProofRequest, l1_coordinator_protocol,
};
use ethrex_blockchain::proof_coordinator::types::{ExecutionProofV1, MAX_PROOF_SIZE, ProofGenId};

use super::proof_types::{
    MIN_REQUIRED_EXECUTION_PROOFS, NewPayloadRequestHeaderV1 as EngineNewPayloadRequestHeaderV1,
    ProofAttributesV1, ProofStatusV1, ProofValidationStatus,
};
use ethrex_common::H256;
use ethrex_common::types::eip8025_ssz;
use ethrex_common::types::requests::{EncodedRequests, compute_requests_hash};
use libssz_merkle::{HashTreeRoot, Sha2Hasher};
use libssz_types::SszList;
use serde_json::Value;
use tracing::{debug, info, warn};

use std::time::Instant;

use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::payload::ExecutionPayload;
use crate::utils::RpcErr;

// ── Helper functions ────────────────────────────────────────────────

/// Build a ProofGenId by hashing block_number and root together.
/// Uses keccak256(block_number_be ++ root) truncated to 8 bytes, avoiding
/// collisions that the previous 4+4 byte scheme was susceptible to.
fn make_proof_gen_id(block_number: u64, root: &H256) -> ProofGenId {
    let mut preimage = [0u8; 40]; // 8 bytes block_number + 32 bytes root
    preimage[..8].copy_from_slice(&block_number.to_be_bytes());
    preimage[8..].copy_from_slice(root.as_bytes());
    let hash = ethrex_common::utils::keccak(preimage);
    let mut id = [0u8; 8];
    id.copy_from_slice(&hash.as_bytes()[..8]);
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
        let coordinator = get_coordinator(&context)?.clone();

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

        // Persist root → block_number mapping in the DB.
        context
            .storage
            .store_root_to_block(new_payload_request_root, block_number)
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        // Generate a ProofGenId from (block_number, root).
        let proof_gen_id = make_proof_gen_id(block_number, &new_payload_request_root);

        // Send the proof request to the coordinator via actor message.
        coordinator
            .send(l1_coordinator_protocol::AddRequest {
                block_number,
                request: Box::new(ProofRequest {
                    proof_gen_id,
                    new_payload_request_root,
                    block,
                    witness,
                    requested_proof_types: self.proof_attributes.proof_types.clone(),
                    created_at: Instant::now(),
                }),
            })
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
            .ok_or_else(|| {
                warn!(root = %root, "Unknown root in verifyExecutionProof");
                RpcErr::InvalidProofFormat(format!(
                    "Unknown new_payload_request_root: {root}. \
                     Call engine_requestProofsV1 first to register the root."
                ))
            })?;

        // NOTE (PoC): In this proof-of-concept we do not cryptographically verify
        // the proof. The exec backend does not generate real ZK proofs — it only
        // re-executes the block. A production implementation must verify the proof
        // against the committed public input before storing it.

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
    // The execution_requests are needed to populate the typed deposit/withdrawal/
    // consolidation request sub-lists inside the SSZ ExecutionPayload.
    let ssz_payload = rpc_payload_to_ssz(payload, execution_requests)?;

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

    let root = ssz_request.hash_tree_root(&Sha2Hasher);
    debug!("SSZ root (full payload): 0x{}", hex::encode(root));

    Ok(root)
}

/// Convert an RPC `ExecutionPayload` into a full SSZ `eip8025_ssz::ExecutionPayload`.
///
/// The `execution_requests` parameter carries the flat EIP-7685 requests
/// (each prefixed with a type byte). These are parsed into the typed
/// deposit/withdrawal/consolidation sub-lists that the CL's
/// `ExecutionPayloadElectra` SSZ container requires.
fn rpc_payload_to_ssz(
    payload: &ExecutionPayload,
    execution_requests: &[Bytes],
) -> Result<eip8025_ssz::ExecutionPayload, String> {
    // Transactions: Vec<EncodedTransaction> -> SszList<SszList<u8, ...>, ...>
    let ssz_txs: Vec<SszList<u8, 1073741824>> = payload
        .transactions
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
        .withdrawals
        .as_deref()
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
    base_fee_bytes[..8].copy_from_slice(&payload.base_fee_per_gas.to_le_bytes());

    // logs_bloom: Bloom (256 bytes) -> SszVector<u8, 256>
    let ssz_logs_bloom: eip8025_ssz::LogsBloom = payload
        .logs_bloom
        .0
        .to_vec()
        .try_into()
        .map_err(|_| "logs_bloom conversion failed".to_string())?;

    // extra_data: Bytes -> SszList<u8, 32>
    let ssz_extra_data: SszList<u8, 32> = payload
        .extra_data
        .to_vec()
        .try_into()
        .map_err(|_| "extra_data too large".to_string())?;

    // Parse the flat EIP-7685 execution_requests into typed sub-lists.
    // Each entry is: type_byte ++ concatenated_fixed_size_requests.
    // The CL's ExecutionPayloadElectra includes these typed fields, so we must
    // populate them for the SSZ hash_tree_root to match what the CL computes.
    let (ssz_deposit_requests, ssz_withdrawal_requests, ssz_consolidation_requests) =
        parse_typed_requests(execution_requests)?;

    Ok(eip8025_ssz::ExecutionPayload {
        parent_hash: payload.parent_hash.0,
        fee_recipient: eip8025_ssz::Bytes20(payload.fee_recipient.0),
        state_root: payload.state_root.0,
        receipts_root: payload.receipts_root.0,
        logs_bloom: ssz_logs_bloom,
        prev_randao: payload.prev_randao.0,
        block_number: payload.block_number,
        gas_limit: payload.gas_limit,
        gas_used: payload.gas_used,
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

    let root = ssz_header.hash_tree_root(&Sha2Hasher);
    info!("SSZ root (json_header): 0x{}", hex::encode(root),);
    Ok(root)
}

// ── EIP-7685 request parsing ────────────────────────────────────────

/// EIP-7685 request type bytes.
const DEPOSIT_TYPE: u8 = 0x00;
const WITHDRAWAL_TYPE: u8 = 0x01;
const CONSOLIDATION_TYPE: u8 = 0x02;

/// Fixed byte sizes for each request type (excluding the type prefix byte).
const DEPOSIT_REQUEST_SIZE: usize = 48 + 32 + 8 + 96 + 8; // 192
const WITHDRAWAL_REQUEST_SIZE: usize = 20 + 48 + 8; // 76
const CONSOLIDATION_REQUEST_SIZE: usize = 20 + 48 + 48; // 116

type ParsedRequests = (
    SszList<eip8025_ssz::DepositRequest, 8192>,
    SszList<eip8025_ssz::WithdrawalRequest, 16>,
    SszList<eip8025_ssz::ConsolidationRequest, 1>,
);

/// Parse flat EIP-7685 `execution_requests` into the three typed SSZ sub-lists
/// expected by `ExecutionPayloadElectra`.
///
/// Each entry in `execution_requests` is `type_byte ++ request1 ++ request2 ++ ...`
/// where each request is a fixed-size byte sequence.
fn parse_typed_requests(execution_requests: &[Bytes]) -> Result<ParsedRequests, String> {
    let mut deposits = Vec::new();
    let mut withdrawals = Vec::new();
    let mut consolidations = Vec::new();

    for entry in execution_requests {
        if entry.is_empty() {
            continue;
        }
        let type_byte = entry[0];
        let data = &entry[1..];

        match type_byte {
            DEPOSIT_TYPE => {
                if data.len() % DEPOSIT_REQUEST_SIZE != 0 {
                    return Err(format!(
                        "Deposit requests data length {} is not a multiple of {DEPOSIT_REQUEST_SIZE}",
                        data.len()
                    ));
                }
                for chunk in data.chunks_exact(DEPOSIT_REQUEST_SIZE) {
                    deposits.push(parse_deposit_request(chunk)?);
                }
            }
            WITHDRAWAL_TYPE => {
                if data.len() % WITHDRAWAL_REQUEST_SIZE != 0 {
                    return Err(format!(
                        "Withdrawal requests data length {} is not a multiple of {WITHDRAWAL_REQUEST_SIZE}",
                        data.len()
                    ));
                }
                for chunk in data.chunks_exact(WITHDRAWAL_REQUEST_SIZE) {
                    withdrawals.push(parse_withdrawal_request(chunk)?);
                }
            }
            CONSOLIDATION_TYPE => {
                if data.len() % CONSOLIDATION_REQUEST_SIZE != 0 {
                    return Err(format!(
                        "Consolidation requests data length {} is not a multiple of {CONSOLIDATION_REQUEST_SIZE}",
                        data.len()
                    ));
                }
                for chunk in data.chunks_exact(CONSOLIDATION_REQUEST_SIZE) {
                    consolidations.push(parse_consolidation_request(chunk)?);
                }
            }
            other => {
                // Unknown type — skip (forward-compatible with future request types).
                debug!("Skipping unknown execution request type: 0x{other:02x}");
            }
        }
    }

    let ssz_deposits = deposits
        .try_into()
        .map_err(|_| "Too many deposit requests".to_string())?;
    let ssz_withdrawals = withdrawals
        .try_into()
        .map_err(|_| "Too many withdrawal requests".to_string())?;
    let ssz_consolidations = consolidations
        .try_into()
        .map_err(|_| "Too many consolidation requests".to_string())?;

    Ok((ssz_deposits, ssz_withdrawals, ssz_consolidations))
}

/// Parse a 192-byte chunk into an SSZ `DepositRequest`.
/// Layout: pubkey(48) ++ withdrawal_credentials(32) ++ amount(8 LE) ++ signature(96) ++ index(8 LE)
fn parse_deposit_request(data: &[u8]) -> Result<eip8025_ssz::DepositRequest, String> {
    let mut pubkey = [0u8; 48];
    pubkey.copy_from_slice(&data[..48]);
    let mut withdrawal_credentials = [0u8; 32];
    withdrawal_credentials.copy_from_slice(&data[48..80]);
    let amount = u64::from_le_bytes(
        data[80..88]
            .try_into()
            .map_err(|_| "deposit amount conversion")?,
    );
    let mut signature = [0u8; 96];
    signature.copy_from_slice(&data[88..184]);
    let index = u64::from_le_bytes(
        data[184..192]
            .try_into()
            .map_err(|_| "deposit index conversion")?,
    );
    Ok(eip8025_ssz::DepositRequest {
        pubkey,
        withdrawal_credentials,
        amount,
        signature,
        index,
    })
}

/// Parse a 76-byte chunk into an SSZ `WithdrawalRequest`.
/// Layout: source_address(20) ++ validator_pubkey(48) ++ amount(8 LE)
fn parse_withdrawal_request(data: &[u8]) -> Result<eip8025_ssz::WithdrawalRequest, String> {
    let mut address = [0u8; 20];
    address.copy_from_slice(&data[..20]);
    let mut validator_pubkey = [0u8; 48];
    validator_pubkey.copy_from_slice(&data[20..68]);
    let amount = u64::from_le_bytes(
        data[68..76]
            .try_into()
            .map_err(|_| "withdrawal amount conversion")?,
    );
    Ok(eip8025_ssz::WithdrawalRequest {
        source_address: eip8025_ssz::Bytes20(address),
        validator_pubkey,
        amount,
    })
}

/// Parse a 116-byte chunk into an SSZ `ConsolidationRequest`.
/// Layout: source_address(20) ++ source_pubkey(48) ++ target_pubkey(48)
fn parse_consolidation_request(data: &[u8]) -> Result<eip8025_ssz::ConsolidationRequest, String> {
    let mut address = [0u8; 20];
    address.copy_from_slice(&data[..20]);
    let mut source_pubkey = [0u8; 48];
    source_pubkey.copy_from_slice(&data[20..68]);
    let mut target_pubkey = [0u8; 48];
    target_pubkey.copy_from_slice(&data[68..116]);
    Ok(eip8025_ssz::ConsolidationRequest {
        source_address: eip8025_ssz::Bytes20(address),
        source_pubkey,
        target_pubkey,
    })
}
