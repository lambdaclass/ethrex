use std::collections::HashMap;
use std::time::Duration;

use tokio::time::timeout;

use bytes::Bytes;
use ethrex_core::{
    types::{AccountState, Block, EMPTY_KECCACK_HASH},
    Address, H256, U256,
};
use ethrex_rlp::decode::RLPDecode;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

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

pub async fn get_block(rpc_url: &str, block_number: usize) -> Result<Block, String> {
    let client = reqwest::Client::new();

    let block_number = format!("0x{block_number:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_getRawBlock",
        "params": [block_number]
    });

    let response = again::retry(|| {
        timeout(
            Duration::from_secs(15),
            client.post(rpc_url).json(request).send(),
        )
    })
    .await
    .map_err(|_| "request timeout")?
    .map_err(|err| err.to_string())?;

    response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| err.to_string())
        .and_then(get_result)
        .and_then(decode_hex)
        .and_then(|encoded_block| {
            Block::decode_unfinished(&encoded_block)
                .map_err(|err| err.to_string())
                .map(|decoded| decoded.0)
        })
}

pub async fn get_account(
    rpc_url: &str,
    block_number: usize,
    address: &Address,
    storage_keys: &[H256],
) -> Result<Account, String> {
    let client = reqwest::Client::new();

    let block_number_str = format!("0x{block_number:x}");
    let address_str = format!("0x{address:x}");
    let storage_keys = storage_keys
        .iter()
        .map(|key| format!("0x{key:x}"))
        .collect::<Vec<String>>();

    let request = &json!(
           {
               "id": 1,
               "jsonrpc": "2.0",
               "method": "eth_getProof",
               "params":[address_str, storage_keys, block_number_str]
           }
    );
    let response = again::retry(|| {
        timeout(
            Duration::from_secs(15),
            client.post(rpc_url).json(request).send(),
        )
    })
    .await
    .map_err(|_| "request timeout")?
    .map_err(|err| err.to_string())?;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AccountProof {
        balance: String,
        code_hash: String,
        nonce: String,
        storage_hash: String,
        storage_proof: Vec<StorageProof>,
        account_proof: Vec<String>,
    }

    #[derive(Deserialize)]
    struct StorageProof {
        key: String,
        value: String,
        proof: Vec<String>,
    }

    let AccountProof {
        balance,
        code_hash,
        nonce,
        storage_hash,
        storage_proof,
        account_proof,
    } = response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| err.to_string())
        .and_then(get_result)?;

    let (storage, storage_proofs) = storage_proof
        .into_iter()
        .map(|proof| -> Result<_, String> {
            let key: H256 = proof
                .key
                .parse()
                .map_err(|_| "failed to parse storage key".to_string())?;
            let value: U256 = proof
                .value
                .parse()
                .map_err(|_| "failed to parse storage value".to_string())?;
            let proofs = proof
                .proof
                .into_iter()
                .map(decode_hex)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(((key, value), proofs))
        })
        .collect::<Result<(HashMap<_, _>, Vec<_>), _>>()?;

    let account_state = AccountState {
        nonce: u64::from_str_radix(nonce.trim_start_matches("0x"), 16)
            .map_err(|_| "failed to parse nonce".to_string())?,
        balance: balance
            .parse()
            .map_err(|_| "failed to parse balance".to_string())?,
        storage_root: storage_hash
            .parse()
            .map_err(|_| "failed to parse storage root".to_string())?,
        code_hash: code_hash
            .parse()
            .map_err(|_| "failed to parse code hash".to_string())?,
    };

    let code = if account_state.code_hash != *EMPTY_KECCACK_HASH {
        Some(get_code(rpc_url, block_number, address).await?)
    } else {
        None
    };

    let account_proof = account_proof
        .into_iter()
        .map(decode_hex)
        .collect::<Result<Vec<_>, String>>()?;

    Ok(Account {
        account_state,
        storage,
        account_proof,
        storage_proofs,
        code,
    })
}

pub async fn get_storage(
    rpc_url: &str,
    block_number: usize,
    address: &Address,
    storage_key: H256,
) -> Result<U256, String> {
    let client = reqwest::Client::new();

    let block_number_str = format!("0x{block_number:x}");
    let address_str = format!("0x{address:x}");
    let storage_key = format!("0x{storage_key:x}");

    let request = &json!(
           {
               "id": 1,
               "jsonrpc": "2.0",
               "method": "eth_getStorageAt",
               "params":[address_str, storage_key, block_number_str]
           }
    );
    let response = again::retry(|| {
        timeout(
            Duration::from_secs(15),
            client.post(rpc_url).json(request).send(),
        )
    })
    .await
    .map_err(|_| "request timeout")?
    .map_err(|err| err.to_string())?;

    response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| err.to_string())
        .and_then(get_result)
}

async fn get_code(rpc_url: &str, block_number: usize, address: &Address) -> Result<Bytes, String> {
    let client = reqwest::Client::new();

    let block_number = format!("0x{block_number:x}");
    let address = format!("0x{address:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getCode",
        "params": [address, block_number]
    });

    let response = again::retry(|| {
        timeout(
            Duration::from_secs(15),
            client.post(rpc_url).json(request).send(),
        )
    })
    .await
    .map_err(|_| "request timeout")?
    .map_err(|err| err.to_string())?;

    response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| err.to_string())
        .and_then(get_result)
        .and_then(decode_hex)
        .map(Bytes::from_owner)
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

#[cfg(test)]
mod test {
    use ethrex_core::Address;

    use super::*;

    const BLOCK_NUMBER: usize = 21315830;
    const RPC_URL: &str = "<to-complete>";
    const VITALIK_ADDR: &str = "d8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

    #[ignore = "needs to manually set rpc url in constant"]
    #[tokio::test]
    async fn get_block_works() {
        get_block(RPC_URL, BLOCK_NUMBER).await.unwrap();
    }

    #[ignore = "needs to manually set rpc url in constant"]
    #[tokio::test]
    async fn get_account_works() {
        get_account(
            RPC_URL,
            BLOCK_NUMBER,
            &Address::from_slice(&hex::decode(VITALIK_ADDR).unwrap()),
            &[],
        )
        .await
        .unwrap();
    }
}
