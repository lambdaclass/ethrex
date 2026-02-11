use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::serde_utils::bytes::deserialize;
use ethrex_common::serde_utils::u64;
use ethrex_common::serde_utils::u256;
use ethrex_common::types::Code;
use ethrex_common::types::{Account, AccountInfo};
use ethrex_common::{Address, U256, types::Fork};
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct RunnerInput {
    pub fork: Fork,
    pub transaction: InputTransaction,
    pub pre: HashMap<Address, InputAccount>,
    #[serde(deserialize_with = "deserialize")]
    pub initial_memory: Bytes,
    #[serde(deserialize_with = "u256::vec::deserialize")]
    pub initial_stack: Vec<U256>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct InputAccount {
    #[serde(deserialize_with = "u256::deser_hex_or_dec_str")]
    pub balance: U256,
    #[serde(deserialize_with = "deserialize")]
    pub code: Bytes,
    #[serde(deserialize_with = "u256::hashmap::deserialize")]
    pub storage: HashMap<U256, U256>,
}

impl From<InputAccount> for Account {
    fn from(account: InputAccount) -> Self {
        let code = Code::from_bytecode(account.code);
        Account {
            info: AccountInfo {
                code_hash: code.hash,
                balance: account.balance,
                nonce: 0,
            },
            code,
            storage: account
                .storage
                .into_iter()
                .map(|(k, v)| (H256::from(k.to_big_endian()), v))
                .collect(),
        }
    }
}

impl Default for InputAccount {
    fn default() -> Self {
        InputAccount {
            balance: high_u256(),
            code: Bytes::new(),
            storage: HashMap::new(),
        }
    }
}

// Super basic transaction data
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct InputTransaction {
    pub to: Option<Address>,
    pub sender: Address,
    #[serde(deserialize_with = "u64::deser_hex_or_dec_str")]
    pub gas_limit: u64,
    #[serde(deserialize_with = "u256::deser_hex_or_dec_str")]
    pub gas_price: U256,
    #[serde(deserialize_with = "u256::deser_hex_or_dec_str")]
    pub value: U256,
    #[serde(deserialize_with = "deserialize")]
    pub data: Bytes,
}

impl Default for InputTransaction {
    fn default() -> Self {
        InputTransaction {
            to: default_recipient(),
            sender: default_sender(),
            gas_limit: high_u64(),
            gas_price: one_u256(),
            value: U256::zero(),
            data: Bytes::new(),
        }
    }
}

impl From<InputTransaction> for ethrex_common::types::LegacyTransaction {
    fn from(tx: InputTransaction) -> Self {
        ethrex_common::types::LegacyTransaction {
            nonce: 0,
            gas_price: tx.gas_price,
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
            ..Default::default()
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
