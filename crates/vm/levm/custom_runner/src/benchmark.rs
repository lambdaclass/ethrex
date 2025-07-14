use crate::deserialize::{
    deserialize_hex_bytes, deserialize_u64_str, deserialize_u256_str,
    deserialize_u256_valued_hashmap, deserialize_u256_vec,
};
use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::types::{Account, AccountInfo, code_hash};
use ethrex_common::{Address, U256, types::Fork};
use serde::Deserialize;
use std::str::FromStr;
use std::{collections::HashMap, u64};

#[derive(Deserialize, Debug)]
pub struct ExecutionInput {
    #[serde(default)]
    pub fork: Fork,
    #[serde(default)]
    pub transaction: BenchTransaction,
    #[serde(default)]
    pub pre: HashMap<Address, BenchAccount>,
    #[serde(default, deserialize_with = "deserialize_hex_bytes")]
    pub initial_memory: Bytes,
    #[serde(default, deserialize_with = "deserialize_u256_vec")]
    pub initial_stack: Vec<U256>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BenchAccount {
    #[serde(default = "high_u256", deserialize_with = "deserialize_u256_str")]
    pub balance: U256,
    #[serde(default, deserialize_with = "deserialize_hex_bytes")]
    pub code: Bytes,
    #[serde(default, deserialize_with = "deserialize_u256_valued_hashmap")]
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
        serde_json::from_str("{}").unwrap()
    }
}

// Super basic transaction data
#[derive(Deserialize, Debug, Clone)]
pub struct BenchTransaction {
    #[serde(default = "default_recipient")]
    pub to: Option<Address>,
    #[serde(default = "default_sender")]
    pub sender: Address,
    #[serde(default = "high_u64", deserialize_with = "deserialize_u64_str")]
    pub gas_limit: u64,
    #[serde(default = "one_u256", deserialize_with = "deserialize_u256_str")]
    pub gas_price: U256,
    #[serde(default, deserialize_with = "deserialize_u256_str")]
    pub value: U256,
    #[serde(default, deserialize_with = "deserialize_hex_bytes")]
    pub data: Bytes,
}

impl Default for BenchTransaction {
    fn default() -> Self {
        // This trick deserializes an empty JSON object.
        // Serde will see all fields are missing and apply the
        // `#[serde(default = "...")]` attributes to build the struct.
        // serde default is necessary on each attribute so that in the JSON
        // we can get away with specifying some attributes only
        serde_json::from_str("{}").unwrap()
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
    u64::from(100_000_000_000u64)
}

pub fn high_u256() -> U256 {
    U256::from(100_000_000_000u64)
}
