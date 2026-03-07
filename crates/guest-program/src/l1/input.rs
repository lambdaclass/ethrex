use ethrex_common::types::{Block, block_execution_witness::ExecutionWitness};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

/// Input for the L1 stateless validation program.
#[derive(Default, Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProgramInput {
    /// Blocks to execute.
    pub blocks: Vec<Block>,
    /// Database containing all the data necessary to execute.
    pub execution_witness: ExecutionWitness,
}

impl ProgramInput {
    /// Creates a new ProgramInput with the given blocks and execution witness.
    pub fn new(blocks: Vec<Block>, execution_witness: ExecutionWitness) -> Self {
        Self {
            blocks,
            execution_witness,
        }
    }
}

// --- EIP-8025 types ---

#[cfg(feature = "eip-8025")]
use bytes::Bytes;
#[cfg(feature = "eip-8025")]
use ethrex_common::{
    Address, Bloom, H256,
    constants::DEFAULT_OMMERS_HASH,
    types::{
        BlockBody, BlockHeader, Transaction, Withdrawal, compute_transactions_root,
        compute_withdrawals_root, requests::compute_requests_hash, requests::EncodedRequests,
    },
};
#[cfg(feature = "eip-8025")]
use ethrex_rlp::error::RLPDecodeError;

/// Execution payload fields matching ExecutionPayloadV3 structure.
/// This is a guest-program-local type that avoids RPC/serde dependencies.
#[cfg(feature = "eip-8025")]
#[derive(Clone, Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ExecutionPayloadData {
    #[rkyv(with = ethrex_common::rkyv_utils::H256Wrapper)]
    pub parent_hash: H256,
    #[rkyv(with = ethrex_common::rkyv_utils::H160Wrapper)]
    pub fee_recipient: Address,
    #[rkyv(with = ethrex_common::rkyv_utils::H256Wrapper)]
    pub state_root: H256,
    #[rkyv(with = ethrex_common::rkyv_utils::H256Wrapper)]
    pub receipts_root: H256,
    #[rkyv(with = ethrex_common::rkyv_utils::BloomWrapper)]
    pub logs_bloom: Bloom,
    #[rkyv(with = ethrex_common::rkyv_utils::H256Wrapper)]
    pub prev_randao: H256,
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    #[rkyv(with = ethrex_common::rkyv_utils::BytesWrapper)]
    pub extra_data: Bytes,
    pub base_fee_per_gas: u64,
    #[rkyv(with = ethrex_common::rkyv_utils::H256Wrapper)]
    pub block_hash: H256,
    /// Transactions in canonical (EIP-2718) encoded form.
    pub transactions: Vec<Vec<u8>>,
    pub withdrawals: Option<Vec<Withdrawal>>,
    pub blob_gas_used: Option<u64>,
    pub excess_blob_gas: Option<u64>,
}

/// NewPayloadRequest for EIP-8025: the full engine API request
/// that gets proven in the zkVM guest program.
#[cfg(feature = "eip-8025")]
#[derive(Clone, Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct NewPayloadRequest {
    pub execution_payload: ExecutionPayloadData,
    /// Versioned hashes stored as raw 32-byte arrays for rkyv compatibility.
    pub versioned_hashes: Vec<[u8; 32]>,
    #[rkyv(with = ethrex_common::rkyv_utils::H256Wrapper)]
    pub parent_beacon_block_root: H256,
    /// Execution requests as raw bytes (each inner Vec is a single encoded request).
    pub execution_requests: Vec<Vec<u8>>,
}

#[cfg(feature = "eip-8025")]
impl NewPayloadRequest {
    /// Convert this NewPayloadRequest into an EL Block, mirroring the
    /// conversion in `crates/networking/rpc/types/payload.rs`.
    pub fn to_block(&self) -> Result<Block, RLPDecodeError> {
        let payload = &self.execution_payload;

        let transactions: Vec<Transaction> = payload
            .transactions
            .iter()
            .map(|raw| Transaction::decode_canonical(raw))
            .collect::<Result<Vec<_>, _>>()?;

        let body = BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: payload.withdrawals.clone(),
        };

        let encoded_requests: Vec<EncodedRequests> = self
            .execution_requests
            .iter()
            .map(|r| EncodedRequests(Bytes::from(r.clone())))
            .collect();
        let requests_hash = compute_requests_hash(&encoded_requests);

        let header = BlockHeader {
            parent_hash: payload.parent_hash,
            ommers_hash: *DEFAULT_OMMERS_HASH,
            coinbase: payload.fee_recipient,
            state_root: payload.state_root,
            transactions_root: compute_transactions_root(&body.transactions),
            receipts_root: payload.receipts_root,
            logs_bloom: payload.logs_bloom,
            difficulty: 0.into(),
            number: payload.block_number,
            gas_limit: payload.gas_limit,
            gas_used: payload.gas_used,
            timestamp: payload.timestamp,
            extra_data: payload.extra_data.clone(),
            prev_randao: payload.prev_randao,
            nonce: 0,
            base_fee_per_gas: Some(payload.base_fee_per_gas),
            withdrawals_root: body
                .withdrawals
                .as_ref()
                .map(|w| compute_withdrawals_root(w)),
            blob_gas_used: payload.blob_gas_used,
            excess_blob_gas: payload.excess_blob_gas,
            parent_beacon_block_root: Some(self.parent_beacon_block_root),
            requests_hash: Some(requests_hash),
            ..Default::default()
        };

        Ok(Block::new(header, body))
    }

    /// Get versioned hashes as H256 values.
    pub fn versioned_hashes_h256(&self) -> Vec<H256> {
        self.versioned_hashes
            .iter()
            .map(|h| H256::from_slice(h))
            .collect()
    }
}

/// EIP-8025 program input: a NewPayloadRequest plus execution witness.
#[cfg(feature = "eip-8025")]
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct Eip8025ProgramInput {
    pub new_payload_request: NewPayloadRequest,
    pub execution_witness: ExecutionWitness,
}
