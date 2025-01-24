use std::collections::HashMap;

use bytes::Bytes;
use ethrex_core::{
    types::{AccountState, Block, EMPTY_KECCACK_HASH},
    Address, H256, U256,
};
use serde::de::DeserializeOwned;

pub mod asynch;
pub mod db;

pub type NodeRLP = Vec<u8>;

#[derive(Clone)]
pub struct Account {
    pub account_state: AccountState,
    pub storage: HashMap<H256, U256>,
    pub account_proof: Vec<NodeRLP>,
    pub storage_proofs: Vec<Vec<NodeRLP>>,
    pub code: Option<Bytes>,
}

fn get_result<T: DeserializeOwned>(response: serde_json::Value) -> Result<T, String> {
    match response.get("result") {
        Some(result) => serde_json::from_value(result.clone()).map_err(|err| err.to_string()),
        None => Err(format!("result not found, response is: {response}")),
    }
}

fn decode_hex(hex: String) -> Result<Vec<u8>, String> {
    let mut trimmed = hex.trim_start_matches("0x").to_string();
    if trimmed.len() % 2 != 0 {
        trimmed = "0".to_string() + &trimmed;
    }
    hex::decode(trimmed).map_err(|err| format!("failed to decode hex string: {err}"))
}
