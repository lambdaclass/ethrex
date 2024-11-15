use crate::deserialize::{
    deserialize_ef_post_value_indexes, deserialize_hex_bytes, deserialize_hex_bytes_vec,
    deserialize_u256_optional_safe, deserialize_u256_safe, deserialize_u256_valued_hashmap_safe,
    deserialize_u256_vec_safe,
};
use bytes::Bytes;
use ethereum_rust_core::{types::TxKind, Address, H256, U256};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug)]
pub struct EFTest {
    pub name: String,
    pub _info: EFTestInfo,
    pub env: EFTestEnv,
    pub post: EFTestPost,
    pub pre: EFTestPre,
    pub transactions: Vec<((usize, usize, usize), EFTestTransaction)>,
}

impl From<&EFTest> for ethereum_rust_levm::db::Db {
    fn from(test: &EFTest) -> Self {
        let mut db = Self::default();
        let mut accounts = Vec::new();
        for (address, pre_value) in &test.pre.0 {
            let storage = pre_value
                .storage
                .clone()
                .into_iter()
                .map(|(k, v)| {
                    let mut key_bytes = [0u8; 32];
                    k.to_big_endian(&mut key_bytes);
                    let storage_slot = ethereum_rust_levm::StorageSlot {
                        original_value: v,
                        current_value: v,
                    };
                    (H256::from_slice(&key_bytes), storage_slot)
                })
                .collect();
            let account = ethereum_rust_levm::Account::new(
                pre_value.balance,
                pre_value.code.clone(),
                pre_value.nonce.as_u64(),
                storage,
            );
            accounts.push((*address, account));
        }
        db.add_accounts(accounts);
        db
    }
}

#[derive(Debug, Deserialize)]
pub struct EFTestInfo {
    pub comment: String,
    #[serde(rename = "filling-rpc-server")]
    pub filling_rpc_server: String,
    #[serde(rename = "filling-tool-version")]
    pub filling_tool_version: String,
    #[serde(rename = "generatedTestHash")]
    pub generated_test_hash: H256,
    #[serde(default)]
    pub labels: Option<HashMap<u64, String>>,
    pub lllcversion: String,
    pub solidity: String,
    pub source: String,
    #[serde(rename = "sourceHash")]
    pub source_hash: H256,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EFTestEnv {
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_base_fee: U256,
    pub current_coinbase: Address,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_difficulty: U256,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_excess_blob_gas: U256,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_gas_limit: U256,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_number: U256,
    pub current_random: H256,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_timestamp: U256,
}

#[derive(Debug, Deserialize, Clone)]
pub enum EFTestPost {
    Cancun(Vec<EFTestPostValue>),
}

impl EFTestPost {
    pub fn values(self) -> Vec<EFTestPostValue> {
        match self {
            EFTestPost::Cancun(v) => v,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct EFTestPostValue {
    #[serde(rename = "expectException")]
    pub expect_exception: Option<String>,
    pub hash: H256,
    #[serde(deserialize_with = "deserialize_ef_post_value_indexes")]
    pub indexes: HashMap<String, U256>,
    pub logs: H256,
    #[serde(deserialize_with = "deserialize_hex_bytes")]
    pub txbytes: Bytes,
}

#[derive(Debug, Deserialize)]
pub struct EFTestPre(pub HashMap<Address, EFTestPreValue>);

#[derive(Debug, Deserialize)]
pub struct EFTestPreValue {
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub balance: U256,
    #[serde(deserialize_with = "deserialize_hex_bytes")]
    pub code: Bytes,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub nonce: U256,
    #[serde(deserialize_with = "deserialize_u256_valued_hashmap_safe")]
    pub storage: HashMap<U256, U256>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EFTestRawTransaction {
    #[serde(deserialize_with = "deserialize_hex_bytes_vec")]
    pub data: Vec<Bytes>,
    #[serde(deserialize_with = "deserialize_u256_vec_safe")]
    pub gas_limit: Vec<U256>,
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub nonce: U256,
    pub secret_key: H256,
    pub sender: Address,
    pub to: TxKind,
    #[serde(deserialize_with = "deserialize_u256_vec_safe")]
    pub value: Vec<U256>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EFTestTransaction {
    pub data: Bytes,
    pub gas_limit: U256,
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub nonce: U256,
    pub secret_key: H256,
    pub sender: Address,
    pub to: TxKind,
    pub value: U256,
}
