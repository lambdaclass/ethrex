//! Convert engine REST SSZ envelopes into ethrex's internal `Block` plus an
//! `EngineCall` enum that selects the right `handle_new_payload_*` helper.

use bytes::Bytes;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::requests::{EncodedRequests, compute_requests_hash};
use ethrex_common::types::{Block, Withdrawal as InternalWithdrawal};
use ethrex_common::{Address, Bloom, H256, U256};
use ethrex_rlp::decode::RLPDecode;

use crate::engine::payload::validate_execution_requests;
use crate::engine_rest::error::ProblemJson;
use crate::engine_rest::types::common::Bytes20;
use crate::engine_rest::types::{amsterdam, cancun, paris, prague, shanghai};
use crate::types::payload::{EncodedTransaction, ExecutionPayload as JsonExecutionPayload};

/// Dispatch tag selecting which existing `handle_new_payload_*` helper to call.
/// Most variant fields are baked into the reconstructed `Block` upstream and are
/// not read again in the final dispatch match — though `V5.raw_bal_hash` is read
/// by the handler's structural BAL check before dispatch (see `handlers::payloads`),
/// and tests inspect the rest. They remain useful for debug logging, so silence
/// dead-code lints rather than drop them.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum EngineCall {
    V1V2,
    V3 {
        parent_beacon_block_root: H256,
    },
    V4 {
        parent_beacon_block_root: H256,
        execution_requests: Vec<EncodedRequests>,
    },
    V5 {
        parent_beacon_block_root: H256,
        execution_requests: Vec<EncodedRequests>,
        raw_bal_hash: Option<H256>,
    },
}

/// Outcome of converting an SSZ envelope: the reconstructed `Block`, the
/// CL-claimed `block_hash` (preserved separately for `validate_block_hash`,
/// since the field is not stored on `Block`), and the dispatch tag.
pub(crate) struct DecodedNewPayload {
    pub block: Block,
    pub expected_block_hash: H256,
    pub call: EngineCall,
    /// Decoded BAL for V5/Amsterdam payloads (None for earlier forks). Passed to
    /// `handle_new_payload_v4` so its `validate_ordering` check runs, matching the
    /// JSON-RPC `engine_newPayloadV5` path.
    pub block_access_list: Option<BlockAccessList>,
}

/// Implemented by each per-fork `ExecutionPayloadEnvelope`.
pub(crate) trait IntoEngineCall {
    fn into_engine_call(self) -> Result<DecodedNewPayload, ProblemJson>;
}

// ── Helper: SSZ base_fee_per_gas (32-byte LE U256) → u64 ─────────────────────

fn base_fee_to_u64(ssz: [u8; 32]) -> Result<u64, ProblemJson> {
    let u256 = U256::from_little_endian(&ssz);
    u64::try_from(u256)
        .map_err(|_| ProblemJson::unprocessable_entity("base_fee_per_gas exceeds u64::MAX"))
}

// ── Helper: SSZ Bytes20 → Address ─────────────────────────────────────────────

fn ssz_address(b: &Bytes20) -> Address {
    Address::from_slice(&b.0)
}

// ── Helper: SSZ Withdrawal → internal Withdrawal ──────────────────────────────

fn ssz_withdrawal(w: &shanghai::Withdrawal) -> InternalWithdrawal {
    InternalWithdrawal {
        index: w.index,
        validator_index: w.validator_index,
        address: ssz_address(&w.address),
        amount: w.amount,
    }
}

// ── Helper: convert execution_requests SSZ list → Vec<EncodedRequests> ────────

fn ssz_requests<const MAX_BYTES: usize, const MAX_COUNT: usize>(
    list: &libssz_types::SszList<libssz_types::SszList<u8, MAX_BYTES>, MAX_COUNT>,
) -> Vec<EncodedRequests> {
    list.iter()
        .map(|r| EncodedRequests(Bytes::from(r.to_vec())))
        .collect()
}

// ── Helper: fill JSON payload from the Cancun/Prague/Amsterdam payload shape ──
// The SSZ `ExecutionPayload` for these forks has the same field set
// (withdrawals + blob fields). We accept them individually to avoid a trait
// boundary that would couple this helper to fork-specific types.

struct CommonCancunFields {
    parent_hash: [u8; 32],
    fee_recipient: Bytes20,
    state_root: [u8; 32],
    receipts_root: [u8; 32],
    logs_bloom: Vec<u8>,
    prev_randao: [u8; 32],
    block_number: u64,
    gas_limit: u64,
    gas_used: u64,
    timestamp: u64,
    extra_data: Vec<u8>,
    base_fee_per_gas: [u8; 32],
    block_hash: [u8; 32],
    transactions: Vec<Vec<u8>>,
    withdrawals: Vec<InternalWithdrawal>,
    blob_gas_used: Option<u64>,
    excess_blob_gas: Option<u64>,
    slot_number: Option<u64>,
}

fn cancun_fields_to_json(f: CommonCancunFields) -> Result<JsonExecutionPayload, ProblemJson> {
    Ok(JsonExecutionPayload {
        parent_hash: H256::from(f.parent_hash),
        fee_recipient: ssz_address(&f.fee_recipient),
        state_root: H256::from(f.state_root),
        receipts_root: H256::from(f.receipts_root),
        logs_bloom: Bloom::from_slice(&f.logs_bloom),
        prev_randao: H256::from(f.prev_randao),
        block_number: f.block_number,
        gas_limit: f.gas_limit,
        gas_used: f.gas_used,
        timestamp: f.timestamp,
        extra_data: Bytes::from(f.extra_data),
        base_fee_per_gas: base_fee_to_u64(f.base_fee_per_gas)?,
        block_hash: H256::from(f.block_hash),
        transactions: f
            .transactions
            .into_iter()
            .map(|t| EncodedTransaction(Bytes::from(t)))
            .collect(),
        withdrawals: Some(f.withdrawals),
        blob_gas_used: f.blob_gas_used,
        excess_blob_gas: f.excess_blob_gas,
        slot_number: f.slot_number,
        block_access_list: None,
    })
}

// ── Paris ─────────────────────────────────────────────────────────────────────

fn paris_payload_to_json(p: paris::ExecutionPayload) -> Result<JsonExecutionPayload, ProblemJson> {
    Ok(JsonExecutionPayload {
        parent_hash: H256::from(p.parent_hash),
        fee_recipient: ssz_address(&p.fee_recipient),
        state_root: H256::from(p.state_root),
        receipts_root: H256::from(p.receipts_root),
        logs_bloom: Bloom::from_slice(&p.logs_bloom[..]),
        prev_randao: H256::from(p.prev_randao),
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: Bytes::from(p.extra_data.to_vec()),
        base_fee_per_gas: base_fee_to_u64(p.base_fee_per_gas)?,
        block_hash: H256::from(p.block_hash),
        transactions: p
            .transactions
            .iter()
            .map(|t| EncodedTransaction(Bytes::from(t.to_vec())))
            .collect(),
        withdrawals: None,
        blob_gas_used: None,
        excess_blob_gas: None,
        slot_number: None,
        block_access_list: None,
    })
}

impl IntoEngineCall for paris::ExecutionPayloadEnvelope {
    fn into_engine_call(self) -> Result<DecodedNewPayload, ProblemJson> {
        let expected_block_hash = H256::from(self.execution_payload.block_hash);
        let json = paris_payload_to_json(self.execution_payload)?;
        let block = json
            .to_block(None, None, None)
            .map_err(|e| ProblemJson::unprocessable_entity(&e.to_string()))?;
        Ok(DecodedNewPayload {
            block,
            expected_block_hash,
            call: EngineCall::V1V2,
            block_access_list: None,
        })
    }
}

// ── Shanghai ──────────────────────────────────────────────────────────────────

fn shanghai_payload_to_json(
    p: shanghai::ExecutionPayload,
) -> Result<JsonExecutionPayload, ProblemJson> {
    let withdrawals: Vec<InternalWithdrawal> = p.withdrawals.iter().map(ssz_withdrawal).collect();
    cancun_fields_to_json(CommonCancunFields {
        parent_hash: p.parent_hash,
        fee_recipient: p.fee_recipient,
        state_root: p.state_root,
        receipts_root: p.receipts_root,
        logs_bloom: p.logs_bloom[..].to_vec(),
        prev_randao: p.prev_randao,
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: p.extra_data.to_vec(),
        base_fee_per_gas: p.base_fee_per_gas,
        block_hash: p.block_hash,
        transactions: p.transactions.iter().map(|t| t.to_vec()).collect(),
        withdrawals,
        blob_gas_used: None,
        excess_blob_gas: None,
        slot_number: None,
    })
}

impl IntoEngineCall for shanghai::ExecutionPayloadEnvelope {
    fn into_engine_call(self) -> Result<DecodedNewPayload, ProblemJson> {
        let expected_block_hash = H256::from(self.execution_payload.block_hash);
        let json = shanghai_payload_to_json(self.execution_payload)?;
        let block = json
            .to_block(None, None, None)
            .map_err(|e| ProblemJson::unprocessable_entity(&e.to_string()))?;
        Ok(DecodedNewPayload {
            block,
            expected_block_hash,
            call: EngineCall::V1V2,
            block_access_list: None,
        })
    }
}

// ── Cancun ────────────────────────────────────────────────────────────────────

fn cancun_payload_to_json(
    p: cancun::ExecutionPayload,
) -> Result<JsonExecutionPayload, ProblemJson> {
    let withdrawals: Vec<InternalWithdrawal> = p.withdrawals.iter().map(ssz_withdrawal).collect();
    cancun_fields_to_json(CommonCancunFields {
        parent_hash: p.parent_hash,
        fee_recipient: p.fee_recipient,
        state_root: p.state_root,
        receipts_root: p.receipts_root,
        logs_bloom: p.logs_bloom[..].to_vec(),
        prev_randao: p.prev_randao,
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: p.extra_data.to_vec(),
        base_fee_per_gas: p.base_fee_per_gas,
        block_hash: p.block_hash,
        transactions: p.transactions.iter().map(|t| t.to_vec()).collect(),
        withdrawals,
        blob_gas_used: Some(p.blob_gas_used),
        excess_blob_gas: Some(p.excess_blob_gas),
        slot_number: None,
    })
}

impl IntoEngineCall for cancun::ExecutionPayloadEnvelope {
    fn into_engine_call(self) -> Result<DecodedNewPayload, ProblemJson> {
        let pbbr = H256::from(self.parent_beacon_block_root);
        let expected_block_hash = H256::from(self.execution_payload.block_hash);
        let json = cancun_payload_to_json(self.execution_payload)?;
        let block = json
            .to_block(Some(pbbr), None, None)
            .map_err(|e| ProblemJson::unprocessable_entity(&e.to_string()))?;
        Ok(DecodedNewPayload {
            block,
            expected_block_hash,
            call: EngineCall::V3 {
                parent_beacon_block_root: pbbr,
            },
            block_access_list: None,
        })
    }
}

// ── Prague ────────────────────────────────────────────────────────────────────

fn prague_payload_to_json(
    p: prague::ExecutionPayload,
) -> Result<JsonExecutionPayload, ProblemJson> {
    let withdrawals: Vec<InternalWithdrawal> = p.withdrawals.iter().map(ssz_withdrawal).collect();
    cancun_fields_to_json(CommonCancunFields {
        parent_hash: p.parent_hash,
        fee_recipient: p.fee_recipient,
        state_root: p.state_root,
        receipts_root: p.receipts_root,
        logs_bloom: p.logs_bloom[..].to_vec(),
        prev_randao: p.prev_randao,
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: p.extra_data.to_vec(),
        base_fee_per_gas: p.base_fee_per_gas,
        block_hash: p.block_hash,
        transactions: p.transactions.iter().map(|t| t.to_vec()).collect(),
        withdrawals,
        blob_gas_used: Some(p.blob_gas_used),
        excess_blob_gas: Some(p.excess_blob_gas),
        slot_number: None,
    })
}

impl IntoEngineCall for prague::ExecutionPayloadEnvelope {
    fn into_engine_call(self) -> Result<DecodedNewPayload, ProblemJson> {
        let pbbr = H256::from(self.parent_beacon_block_root);
        let expected_block_hash = H256::from(self.execution_payload.block_hash);
        let execution_requests = ssz_requests(&self.execution_requests);
        // Spec: execution_requests MUST be ordered ascending by request_type and
        // each entry MUST be non-empty. Mirror the JSON-RPC V4 path.
        validate_execution_requests(&execution_requests)
            .map_err(|err| ProblemJson::bad_request(&err.to_string()))?;
        let requests_hash = compute_requests_hash(&execution_requests);
        let json = prague_payload_to_json(self.execution_payload)?;
        let block = json
            .to_block(Some(pbbr), Some(requests_hash), None)
            .map_err(|e| ProblemJson::unprocessable_entity(&e.to_string()))?;
        Ok(DecodedNewPayload {
            block,
            expected_block_hash,
            call: EngineCall::V4 {
                parent_beacon_block_root: pbbr,
                execution_requests,
            },
            block_access_list: None,
        })
    }
}

// ── Amsterdam ─────────────────────────────────────────────────────────────────

fn amsterdam_payload_to_json(
    p: amsterdam::ExecutionPayload,
) -> Result<(JsonExecutionPayload, Option<H256>, Option<BlockAccessList>), ProblemJson> {
    let withdrawals: Vec<InternalWithdrawal> = p.withdrawals.iter().map(ssz_withdrawal).collect();
    // The block_access_list field carries the RLP-encoded BAL. Hash the raw bytes
    // for the header's `block_access_list_hash`, and decode the BAL itself so the
    // caller can run `validate_ordering` (matching the JSON-RPC V5 path).
    let bal_bytes = p.block_access_list.to_vec();
    let (raw_bal_hash, block_access_list) = if bal_bytes.is_empty() {
        (None, None)
    } else {
        let hash = ethrex_common::utils::keccak(&bal_bytes);
        let bal = BlockAccessList::decode(&bal_bytes).map_err(|err| {
            ProblemJson::bad_request(&format!("invalid block_access_list RLP: {err}"))
        })?;
        (Some(hash), Some(bal))
    };
    let json = cancun_fields_to_json(CommonCancunFields {
        parent_hash: p.parent_hash,
        fee_recipient: p.fee_recipient,
        state_root: p.state_root,
        receipts_root: p.receipts_root,
        logs_bloom: p.logs_bloom[..].to_vec(),
        prev_randao: p.prev_randao,
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: p.extra_data.to_vec(),
        base_fee_per_gas: p.base_fee_per_gas,
        block_hash: p.block_hash,
        transactions: p.transactions.iter().map(|t| t.to_vec()).collect(),
        withdrawals,
        blob_gas_used: Some(p.blob_gas_used),
        excess_blob_gas: Some(p.excess_blob_gas),
        slot_number: Some(p.slot_number),
    })?;
    Ok((json, raw_bal_hash, block_access_list))
}

impl IntoEngineCall for amsterdam::ExecutionPayloadEnvelope {
    fn into_engine_call(self) -> Result<DecodedNewPayload, ProblemJson> {
        let pbbr = H256::from(self.parent_beacon_block_root);
        let expected_block_hash = H256::from(self.execution_payload.block_hash);
        let execution_requests = ssz_requests(&self.execution_requests);
        // Spec: execution_requests MUST be ordered ascending by request_type and
        // each entry MUST be non-empty. Mirror the JSON-RPC V5 path.
        validate_execution_requests(&execution_requests)
            .map_err(|err| ProblemJson::bad_request(&err.to_string()))?;
        let requests_hash = compute_requests_hash(&execution_requests);
        let (json, raw_bal_hash, block_access_list) =
            amsterdam_payload_to_json(self.execution_payload)?;
        let block = json
            .to_block(Some(pbbr), Some(requests_hash), raw_bal_hash)
            .map_err(|e| ProblemJson::unprocessable_entity(&e.to_string()))?;
        Ok(DecodedNewPayload {
            block,
            expected_block_hash,
            call: EngineCall::V5 {
                parent_beacon_block_root: pbbr,
                execution_requests,
                raw_bal_hash,
            },
            block_access_list,
        })
    }
}
