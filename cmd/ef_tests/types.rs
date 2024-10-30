use bytes::Bytes;
use ethereum_rust_core::types::{
    code_hash, Account as ethereum_rustAccount, AccountInfo, Block as CoreBlock, BlockBody,
    EIP1559Transaction, EIP2930Transaction, EIP4844Transaction, LegacyTransaction,
    Transaction as ethereum_rustTransaction, TxKind,
};
use ethereum_rust_core::types::{Genesis, GenesisAccount, Withdrawal};
use ethereum_rust_core::{types::BlockHeader, Address, Bloom, H256, H64, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::network::Network;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestUnit {
    #[serde(default, rename = "_info")]
    pub info: Option<serde_json::Value>,
    pub blocks: Vec<BlockWithRLP>,
    pub genesis_block_header: Header,
    #[serde(rename = "genesisRLP", with = "ethereum_rust_core::serde_utils::bytes")]
    pub genesis_rlp: Bytes,
    pub lastblockhash: H256,
    pub network: Network,
    pub post_state: HashMap<Address, Account>,
    pub pre: HashMap<Address, Account>,
    pub seal_engine: serde_json::Value,
}

impl TestUnit {
    /// Checks wether a test has a block where the inner_block is none.
    /// These tests only check for failures in decoding invalid rlp and expect an exception.
    pub fn is_rlp_only_test(&self) -> bool {
        let mut is_rlp_only = false;
        for block in self.blocks.iter() {
            is_rlp_only = is_rlp_only || block.block().is_none();
        }
        is_rlp_only
    }

    pub fn get_genesis(&self) -> Genesis {
        Genesis {
            config: *self.network.chain_config(),
            alloc: self
                .pre
                .clone()
                .into_iter()
                .map(|(key, val)| (key, val.into()))
                .collect(),
            coinbase: self.genesis_block_header.coinbase,
            difficulty: self.genesis_block_header.difficulty,
            extra_data: self.genesis_block_header.extra_data.clone(),
            gas_limit: self.genesis_block_header.gas_limit.as_u64(),
            nonce: self.genesis_block_header.nonce.to_low_u64_be(),
            mix_hash: self.genesis_block_header.mix_hash,
            timestamp: self.genesis_block_header.timestamp.as_u64(),
            base_fee_per_gas: self
                .genesis_block_header
                .base_fee_per_gas
                .map(|v| v.as_u64()),
            blob_gas_used: self.genesis_block_header.blob_gas_used.map(|v| v.as_u64()),
            excess_blob_gas: self
                .genesis_block_header
                .excess_blob_gas
                .map(|v| v.as_u64()),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct Account {
    pub balance: U256,
    #[serde(with = "ethereum_rust_core::serde_utils::bytes")]
    pub code: Bytes,
    pub nonce: U256,
    pub storage: HashMap<U256, U256>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Env {
    pub current_coinbase: Address,
    pub current_difficulty: U256,
    pub current_gas_limit: U256,
    pub current_number: U256,
    pub current_timestamp: U256,
    pub current_base_fee: Option<U256>,
    pub previous_hash: Option<H256>,
    pub current_random: Option<H256>,
    pub current_beacon_root: Option<H256>,
    pub current_withdrawals_root: Option<H256>,
    pub parent_blob_gas_used: Option<U256>,
    pub parent_excess_blob_gas: Option<U256>,
    pub current_excess_blob_gas: Option<U256>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessListItem {
    pub address: Address,
    pub storage_keys: Vec<H256>,
}

pub type AccessList = Vec<AccessListItem>;

#[derive(Debug, PartialEq, Eq, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub bloom: Bloom,
    pub coinbase: Address,
    pub difficulty: U256,
    #[serde(with = "ethereum_rust_core::serde_utils::bytes")]
    pub extra_data: Bytes,
    pub gas_limit: U256,
    pub gas_used: U256,
    pub hash: H256,
    pub mix_hash: H256,
    pub nonce: H64,
    pub number: U256,
    pub parent_hash: H256,
    pub receipt_trie: H256,
    pub state_root: H256,
    pub timestamp: U256,
    pub transactions_trie: H256,
    pub uncle_hash: H256,
    pub base_fee_per_gas: Option<U256>,
    pub withdrawals_root: Option<H256>,
    pub blob_gas_used: Option<U256>,
    pub excess_blob_gas: Option<U256>,
    pub parent_beacon_block_root: Option<H256>,
    pub requests_root: Option<H256>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BlockWithRLP {
    #[serde(with = "ethereum_rust_core::serde_utils::bytes")]
    pub rlp: Bytes,
    #[serde(flatten)]
    inner: Option<BlockInner>,
    pub expect_exception: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
#[serde(untagged)]
pub enum BlockInner {
    Block(Block),
    DecodedRLP(DecodedRLPBlock),
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
pub struct DecodedRLPBlock {
    rlp_decoded: Block,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub block_header: Header,
    #[serde(default)]
    pub transactions: Vec<Transaction>,
    #[serde(default)]
    pub uncle_headers: Vec<Header>,
    pub withdrawals: Option<Vec<Withdrawal>>,
}

impl BlockWithRLP {
    pub fn block(&self) -> Option<&Block> {
        match self.inner {
            Some(BlockInner::Block(ref block)) => Some(block),
            Some(BlockInner::DecodedRLP(ref decoded)) => Some(&decoded.rlp_decoded),
            None => None,
        }
    }
}
impl From<Block> for CoreBlock {
    fn from(val: Block) -> Self {
        CoreBlock::new(
            val.block_header.into(),
            BlockBody {
                transactions: val.transactions.iter().map(|t| t.clone().into()).collect(),
                ommers: val.uncle_headers.iter().map(|h| h.clone().into()).collect(),
                withdrawals: val.withdrawals,
            },
        )
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    #[serde(rename = "type")]
    pub transaction_type: Option<U256>,
    #[serde(with = "ethereum_rust_core::serde_utils::bytes")]
    pub data: Bytes,
    pub gas_limit: U256,
    pub gas_price: Option<U256>,
    pub nonce: U256,
    pub r: U256,
    pub s: U256,
    pub v: U256,
    pub value: U256,
    pub chain_id: Option<U256>,
    pub access_list: Option<AccessList>,
    pub max_fee_per_gas: Option<U256>,
    pub max_fee_per_blob_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub blob_versioned_hashes: Option<Vec<H256>>,
    pub hash: Option<H256>,
    pub sender: Address,
    pub to: TxKind,
}

// Conversions between EFtests & ethereum_rust types
impl From<Header> for BlockHeader {
    fn from(val: Header) -> Self {
        BlockHeader {
            parent_hash: val.parent_hash,
            ommers_hash: val.uncle_hash,
            coinbase: val.coinbase,
            state_root: val.state_root,
            transactions_root: val.transactions_trie,
            receipts_root: val.receipt_trie,
            logs_bloom: val.bloom,
            difficulty: val.difficulty,
            number: val.number.as_u64(),
            gas_limit: val.gas_limit.as_u64(),
            gas_used: val.gas_used.as_u64(),
            timestamp: val.timestamp.as_u64(),
            extra_data: val.extra_data,
            prev_randao: val.mix_hash,
            nonce: val.nonce.to_low_u64_be(),
            base_fee_per_gas: val.base_fee_per_gas.map(|v| v.as_u64()),
            withdrawals_root: val.withdrawals_root,
            blob_gas_used: val.blob_gas_used.map(|x| x.as_u64()),
            excess_blob_gas: val.excess_blob_gas.map(|x| x.as_u64()),
            parent_beacon_block_root: val.parent_beacon_block_root,
        }
    }
}

impl From<Transaction> for ethereum_rustTransaction {
    fn from(val: Transaction) -> Self {
        match val.transaction_type {
            Some(tx_type) => match tx_type.as_u64() {
                0 => ethereum_rustTransaction::LegacyTransaction(val.into()),
                1 => ethereum_rustTransaction::EIP2930Transaction(val.into()),
                2 => ethereum_rustTransaction::EIP1559Transaction(val.into()),
                3 => ethereum_rustTransaction::EIP4844Transaction(val.into()),
                _ => unimplemented!(),
            },
            None => ethereum_rustTransaction::LegacyTransaction(val.into()),
        }
    }
}

impl From<Transaction> for EIP1559Transaction {
    fn from(val: Transaction) -> Self {
        EIP1559Transaction {
            // Note: gas_price is not used in this conversion as it is not part of EIP1559Transaction, this could be a problem
            chain_id: val.chain_id.map(|id| id.as_u64()).unwrap_or(1 /*mainnet*/), // TODO: Consider converting this into Option
            nonce: val.nonce.as_u64(),
            max_priority_fee_per_gas: val.max_priority_fee_per_gas.unwrap_or_default().as_u64(), // TODO: Consider converting this into Option
            max_fee_per_gas: val
                .max_fee_per_gas
                .unwrap_or(val.gas_price.unwrap_or_default())
                .as_u64(), // TODO: Consider converting this into Option
            gas_limit: val.gas_limit.as_u64(),
            // to: match val.to {
            //     zero if zero == H160::zero() => TxKind::Create,
            //     _ => TxKind::Call(val.to),
            // },
            to: val.to,
            value: val.value,
            data: val.data,
            access_list: val
                .access_list
                .unwrap_or_default()
                .into_iter()
                .map(|item| (item.address, item.storage_keys))
                .collect(),
            signature_y_parity: !val.v.is_zero(),
            signature_r: val.r,
            signature_s: val.s,
        }
    }
}

impl From<Transaction> for EIP4844Transaction {
    fn from(val: Transaction) -> Self {
        EIP4844Transaction {
            chain_id: val.chain_id.map(|id: U256| id.as_u64()).unwrap_or(1), // TODO: Consider converting this into Option
            nonce: val.nonce.as_u64(),
            max_priority_fee_per_gas: val.max_priority_fee_per_gas.unwrap_or_default().as_u64(), // TODO: Consider converting this into Option
            max_fee_per_gas: val
                .max_fee_per_gas
                .unwrap_or(val.gas_price.unwrap_or_default())
                .as_u64(),
            gas: val.gas_limit.as_u64(),
            to: match val.to {
                TxKind::Call(address) => address,
                TxKind::Create => panic!("EIP4844Transaction cannot be contract creation"),
            },
            value: val.value,
            data: val.data,
            access_list: val
                .access_list
                .unwrap_or_default()
                .into_iter()
                .map(|a| (a.address, a.storage_keys))
                .collect(),
            max_fee_per_blob_gas: val.max_fee_per_blob_gas.unwrap(),
            blob_versioned_hashes: val.blob_versioned_hashes.unwrap_or_default(),
            signature_y_parity: !val.v.is_zero(),
            signature_r: val.r,
            signature_s: val.s,
        }
    }
}

impl From<Transaction> for LegacyTransaction {
    fn from(val: Transaction) -> Self {
        LegacyTransaction {
            nonce: val.nonce.as_u64(),
            gas_price: val.gas_price.unwrap_or_default().as_u64(), // TODO: Consider converting this into Option
            gas: val.gas_limit.as_u64(),
            // to: match val.to {
            //     zero if zero == H160::zero() => TxKind::Create,
            //     _ => TxKind::Call(val.to),
            // },
            to: val.to,
            value: val.value,
            data: val.data,
            v: val.v,
            r: val.r,
            s: val.s,
        }
    }
}

impl From<Transaction> for EIP2930Transaction {
    fn from(val: Transaction) -> Self {
        EIP2930Transaction {
            chain_id: val.chain_id.map(|id: U256| id.as_u64()).unwrap_or(1),
            nonce: val.nonce.as_u64(),
            gas_price: val.gas_price.unwrap_or_default().as_u64(),
            gas_limit: val.gas_limit.as_u64(),
            // to: match val.to {
            //     zero if zero == H160::zero() => TxKind::Create,
            //     _ => TxKind::Call(val.to),
            // },
            to: val.to,
            value: val.value,
            data: val.data,
            access_list: val
                .access_list
                .unwrap_or_default()
                .into_iter()
                .map(|a| (a.address, a.storage_keys))
                .collect(),
            signature_y_parity: !val.v.is_zero(),
            signature_r: val.r,
            signature_s: val.s,
        }
    }
}

impl From<Account> for ethereum_rustAccount {
    fn from(val: Account) -> Self {
        ethereum_rustAccount {
            info: AccountInfo {
                code_hash: code_hash(&val.code),
                balance: val.balance,
                nonce: val.nonce.as_u64(),
            },
            code: val.code,
            storage: val
                .storage
                .into_iter()
                .map(|(k, v)| {
                    let mut k_bytes = [0; 32];
                    k.to_big_endian(&mut k_bytes);
                    (H256(k_bytes), v)
                })
                .collect(),
        }
    }
}

impl From<Account> for GenesisAccount {
    fn from(val: Account) -> Self {
        GenesisAccount {
            code: val.code,
            storage: val
                .storage
                .into_iter()
                .map(|(k, v)| {
                    let mut k_bytes = [0; 32];
                    k.to_big_endian(&mut k_bytes);
                    (H256(k_bytes), v)
                })
                .collect(),
            balance: val.balance,
            nonce: val.nonce.as_u64(),
        }
    }
}
