use crate::deserialize::{
    deserialize_hex_bytes, deserialize_u256_str, deserialize_u256_valued_hashmap,
    deserialize_u256_vec, deserialize_u64_str,
};
use bytes::Bytes;
use ethrex_common::types::{code_hash, Account, AccountInfo};
use ethrex_common::H256;
use ethrex_common::{types::Fork, Address, U256};
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct RunnerInput {
    pub fork: Fork,
    pub transaction: BenchTransaction,
    pub pre: HashMap<Address, BenchAccount>,
    #[serde(deserialize_with = "deserialize_hex_bytes")]
    pub initial_memory: Bytes,
    #[serde(deserialize_with = "deserialize_u256_vec")]
    pub initial_stack: Vec<U256>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct BenchAccount {
    #[serde(deserialize_with = "deserialize_u256_str")]
    pub balance: U256,
    #[serde(deserialize_with = "deserialize_hex_bytes")]
    pub code: Bytes,
    #[serde(deserialize_with = "deserialize_u256_valued_hashmap")]
    pub storage: HashMap<U256, U256>,
}

impl From<BenchAccount> for Account {
    fn from(account: BenchAccount) -> Self {
        Account {
            info: AccountInfo {
                code_hash: code_hash(&account.code),
                balance: account.balance,
                nonce: 0,
            },
            code: account.code,
            storage: account
                .storage
                .into_iter()
                .map(|(k, v)| (H256::from(k.to_big_endian()), v))
                .collect(),
        }
    }
}

impl Default for BenchAccount {
    fn default() -> Self {
        BenchAccount {
            balance: high_u256(),
            code: Bytes::new(),
            storage: HashMap::new(),
        }
    }
}

// Super basic transaction data
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct BenchTransaction {
    pub to: Option<Address>,
    pub sender: Address,
    #[serde(deserialize_with = "deserialize_u64_str")]
    pub gas_limit: u64,
    #[serde(deserialize_with = "deserialize_u256_str")]
    pub gas_price: U256,
    #[serde(deserialize_with = "deserialize_u256_str")]
    pub value: U256,
    #[serde(deserialize_with = "deserialize_hex_bytes")]
    pub data: Bytes,
}

impl Default for BenchTransaction {
    fn default() -> Self {
        BenchTransaction {
            to: default_recipient(),
            sender: default_sender(),
            gas_limit: high_u64(),
            gas_price: one_u256(),
            value: U256::zero(),
            data: Bytes::new(),
        }
    }
}

impl From<BenchTransaction> for ethrex_common::types::LegacyTransaction {
    fn from(tx: BenchTransaction) -> Self {
        ethrex_common::types::LegacyTransaction {
            nonce: 0,
            gas_price: tx.gas_price.try_into().unwrap(),
            gas: tx.gas_limit,
            to: match tx.to {
                Some(address) => ethrex_common::types::TxKind::Call(address),
                None => ethrex_common::types::TxKind::Create,
            },
            value: tx.value,
            data: tx.data,
            v: U256::zero(),
            r: U256::zero(),
            s: U256::zero(),
        }
    }
}

pub fn default_sender() -> Address {
    Address::from_str("0x000000000000000000000000000000000000dead").unwrap()
}

pub fn default_recipient() -> Option<Address> {
    Some(Address::from_str("0x000000000000000000000000000000000000beef").unwrap())
}

pub fn one_u256() -> U256 {
    U256::one()
}

pub fn high_u64() -> u64 {
    100_000_000_000
}

pub fn high_u256() -> U256 {
    U256::from(100_000_000_000u64)
}
