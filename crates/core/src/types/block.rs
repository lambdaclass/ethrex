use crate::{rlp::encode::RLPEncode, Address, H256, U256};
use bytes::Bytes;

use super::Transaction;

pub type BlockNumber = u64;
pub type Bloom = [u8; 256];

/// Header part of a block on the chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockHeader {
    pub parent_hash: H256,
    pub ommers_hash: H256, // ommer = uncle
    pub coinbase: Address,
    pub state_root: H256,
    pub transactions_root: H256,
    pub receipt_root: H256,
    pub logs_bloom: Bloom,
    pub difficulty: U256,
    pub number: BlockNumber,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: Bytes,
    pub prev_randao: H256,
    pub nonce: u64,
    pub base_fee_per_gas: u64,
    pub withdrawals_root: H256,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
    pub parent_beacon_block_root: H256,
}

impl RLPEncode for BlockHeader {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        self.parent_hash.encode(buf);
        self.ommers_hash.encode(buf);
        self.coinbase.encode(buf);
        self.state_root.encode(buf);
        self.transactions_root.encode(buf);
        self.receipt_root.encode(buf);
        self.logs_bloom.encode(buf);
        self.difficulty.encode(buf);
        self.number.encode(buf);
        self.gas_limit.encode(buf);
        self.gas_used.encode(buf);
        self.timestamp.encode(buf);
        self.extra_data.encode(buf);
        self.prev_randao.encode(buf);
        self.nonce.encode(buf);
        self.base_fee_per_gas.encode(buf);
        self.withdrawals_root.encode(buf);
        self.blob_gas_used.encode(buf);
        self.excess_blob_gas.encode(buf);
        self.parent_beacon_block_root.encode(buf);
    }
}

// The body of a block on the chain
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    transactions: Vec<Transaction>,
    ommers: Vec<BlockHeader>,
    withdrawals: Vec<Withdrawal>,
}

impl RLPEncode for Body {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        self.transactions.encode(buf);
        self.ommers.encode(buf);
        self.withdrawals.encode(buf);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Withdrawal {
    index: u64,
    validator_index: u64,
    address: Address,
    amount: U256,
}

impl RLPEncode for Withdrawal {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        self.index.encode(buf);
        self.validator_index.encode(buf);
        self.address.encode(buf);
        self.amount.encode(buf);
    }
}
