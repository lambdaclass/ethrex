//! SSZ containers for EIP-8025 hash_tree_root computation inside the guest program.
//!
//! These types duplicate the SSZ definitions from `crates/blockchain/proof_engine/ssz.rs`
//! because the guest program cannot depend on `ethrex-blockchain`.

use ssz_rs::prelude::*;

use super::input::{ExecutionPayloadData, NewPayloadRequest};
use ethrex_common::types::Withdrawal;

// Max list lengths from the consensus spec (Electra/Fulu).
const MAX_TRANSACTIONS_PER_PAYLOAD: usize = 1_048_576;
const MAX_WITHDRAWALS_PER_PAYLOAD: usize = 16;
const MAX_BYTES_PER_TRANSACTION: usize = 1_073_741_824; // 2^30
const MAX_EXTRA_DATA_BYTES: usize = 32;
const MAX_BLOB_COMMITMENTS_PER_BLOCK: usize = 4096;
const MAX_REQUEST_DATA_BYTES: usize = 8_388_608; // 2^23
const MAX_EXECUTION_REQUESTS: usize = 16;
const BYTES_PER_LOGS_BLOOM: usize = 256;

#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszExecutionPayload {
    pub parent_hash: [u8; 32],
    pub fee_recipient: [u8; 20],
    pub state_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub logs_bloom: Vector<u8, BYTES_PER_LOGS_BLOOM>,
    pub prev_randao: [u8; 32],
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: List<u8, MAX_EXTRA_DATA_BYTES>,
    pub base_fee_per_gas: U256,
    pub block_hash: [u8; 32],
    pub transactions: List<List<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
    pub withdrawals: List<SszWithdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszWithdrawal {
    pub index: u64,
    pub validator_index: u64,
    pub address: [u8; 20],
    pub amount: u64,
}

pub type SszExecutionRequests = List<List<u8, MAX_REQUEST_DATA_BYTES>, MAX_EXECUTION_REQUESTS>;

#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszNewPayloadRequest {
    pub execution_payload: SszExecutionPayload,
    pub versioned_hashes: List<[u8; 32], MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub parent_beacon_block_root: [u8; 32],
    pub execution_requests: SszExecutionRequests,
}

impl From<&Withdrawal> for SszWithdrawal {
    fn from(w: &Withdrawal) -> Self {
        SszWithdrawal {
            index: w.index,
            validator_index: w.validator_index,
            address: w.address.to_fixed_bytes(),
            amount: w.amount,
        }
    }
}

impl From<&ExecutionPayloadData> for SszExecutionPayload {
    fn from(p: &ExecutionPayloadData) -> Self {
        let mut logs_bloom = Vector::<u8, BYTES_PER_LOGS_BLOOM>::default();
        for (i, byte) in p.logs_bloom.as_bytes().iter().enumerate() {
            logs_bloom[i] = *byte;
        }

        let mut extra_data = List::<u8, MAX_EXTRA_DATA_BYTES>::default();
        for byte in p.extra_data.iter() {
            extra_data.push(*byte);
        }

        let base_fee = {
            let mut buf = [0u8; 32];
            // ssz_rs U256 is little-endian
            buf[..8].copy_from_slice(&p.base_fee_per_gas.to_le_bytes());
            U256::from_bytes_le(buf)
        };

        let mut transactions = List::default();
        for tx in &p.transactions {
            let mut ssz_tx = List::<u8, MAX_BYTES_PER_TRANSACTION>::default();
            for byte in tx {
                ssz_tx.push(*byte);
            }
            transactions.push(ssz_tx);
        }

        let mut withdrawals = List::default();
        if let Some(ws) = &p.withdrawals {
            for w in ws {
                withdrawals.push(SszWithdrawal::from(w));
            }
        }

        SszExecutionPayload {
            parent_hash: p.parent_hash.to_fixed_bytes(),
            fee_recipient: p.fee_recipient.to_fixed_bytes(),
            state_root: p.state_root.to_fixed_bytes(),
            receipts_root: p.receipts_root.to_fixed_bytes(),
            logs_bloom,
            prev_randao: p.prev_randao.to_fixed_bytes(),
            block_number: p.block_number,
            gas_limit: p.gas_limit,
            gas_used: p.gas_used,
            timestamp: p.timestamp,
            extra_data,
            base_fee_per_gas: base_fee,
            block_hash: p.block_hash.to_fixed_bytes(),
            transactions,
            withdrawals,
            blob_gas_used: p.blob_gas_used.unwrap_or(0),
            excess_blob_gas: p.excess_blob_gas.unwrap_or(0),
        }
    }
}

impl From<&NewPayloadRequest> for SszNewPayloadRequest {
    fn from(req: &NewPayloadRequest) -> Self {
        let mut versioned_hashes = List::default();
        for h in &req.versioned_hashes {
            versioned_hashes.push(*h);
        }

        let mut execution_requests: SszExecutionRequests = List::default();
        for r in &req.execution_requests {
            let mut ssz_req = List::<u8, MAX_REQUEST_DATA_BYTES>::default();
            for byte in r {
                ssz_req.push(*byte);
            }
            execution_requests.push(ssz_req);
        }

        SszNewPayloadRequest {
            execution_payload: SszExecutionPayload::from(&req.execution_payload),
            versioned_hashes,
            parent_beacon_block_root: req.parent_beacon_block_root.to_fixed_bytes(),
            execution_requests,
        }
    }
}

/// Compute hash_tree_root of a NewPayloadRequest.
pub fn compute_new_payload_request_root(
    request: &NewPayloadRequest,
) -> Result<[u8; 32], MerkleizationError> {
    let mut ssz_req = SszNewPayloadRequest::from(request);
    let root = ssz_req.hash_tree_root()?;
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(root.as_ref());
    Ok(bytes)
}
