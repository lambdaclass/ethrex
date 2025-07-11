use crate::deserialize::{
    deserialize_hex_bytes, deserialize_u64_str, deserialize_u256_str, deserialize_u256_vec,
};
use bytes::Bytes;
use ethrex_common::{Address, H160, H256, U256, types::Fork};
use serde::Deserialize;
use std::{collections::HashMap, u64};

const DEFAULT_SENDER: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x99,
]);
const DEFAULT_CONTRACT: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x42,
]);

#[derive(Deserialize, Debug)]
pub struct ExecutionInput {
    #[serde(default)]
    pub fork: Fork,
    pub transaction: Transaction,
    #[serde(default)]
    pub pre: HashMap<Address, Account>,
    #[serde(default, deserialize_with = "deserialize_hex_bytes")]
    pub initial_memory: Bytes,
    #[serde(default, deserialize_with = "deserialize_u256_vec")]
    pub initial_stack: Vec<U256>,
}

#[derive(Deserialize, Debug)]
pub struct Account {
    #[serde(default = "high_u256", deserialize_with = "deserialize_u256_str")]
    pub balance: U256,
    #[serde(default, deserialize_with = "deserialize_hex_bytes")]
    pub code: Bytes,
    pub storage: HashMap<H256, U256>,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            balance: U256::from(1_000_000_000_000u64),
            code: Bytes::new(),
            storage: HashMap::new(),
        }
    }
}

// Super basic transaction data
#[derive(Deserialize, Debug)]
pub struct Transaction {
    #[serde(default = "default_recipient")]
    pub to: Option<Address>,
    #[serde(default = "default_sender")]
    pub sender: Address,
    #[serde(default = "high_u64", deserialize_with = "deserialize_u64_str")]
    pub gas_limit: u64,
    #[serde(default = "one_u64", deserialize_with = "deserialize_u64_str")]
    pub gas_price: u64,
    #[serde(default, deserialize_with = "deserialize_u256_str")]
    pub value: U256,
    #[serde(default, deserialize_with = "deserialize_hex_bytes")]
    pub data: Bytes,
}

fn default_sender() -> Address {
    DEFAULT_SENDER
}

fn default_recipient() -> Option<Address> {
    Some(DEFAULT_CONTRACT)
}

fn one_u64() -> u64 {
    1
}

fn one_u256() -> U256 {
    U256::one()
}

fn high_u64() -> u64 {
    u64::from(100_000_000_000u64)
}

fn high_u256() -> U256 {
    U256::from(100_000_000_000u64)
}

impl Default for Transaction {
    fn default() -> Self {
        Self {
            to: Some(DEFAULT_CONTRACT),
            sender: DEFAULT_SENDER,
            gas_limit: u64::MAX,
            gas_price: 1,
            value: U256::zero(),
            data: Bytes::new(),
        }
    }
}
