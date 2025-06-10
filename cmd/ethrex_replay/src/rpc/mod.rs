use std::collections::HashMap;
use std::time::Duration;

use again::{RetryPolicy, Task};

use bytes::Bytes;
use ethrex_common::{
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, Block},
    Address, H256, U256,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::hash_address;
use ethrex_trie::Trie;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

use lazy_static::lazy_static;

pub mod db;

pub type NodeRLP = Vec<u8>;

lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

#[derive(Clone, Debug)]
pub enum Account {
    Existing {
        account_state: AccountState,
        storage: HashMap<H256, U256>,
        account_proof: Vec<NodeRLP>,
        storage_proofs: HashMap<H256, Vec<NodeRLP>>,
        code: Option<Bytes>,
    },
    NonExisting {
        account_proof: Vec<NodeRLP>,
        storage_proofs: HashMap<H256, Vec<NodeRLP>>,
    },
}

impl Account {
    pub fn get_account_proof(&self) -> &Vec<NodeRLP> {
        match self {
            Account::Existing { account_proof, .. } => account_proof,
            Account::NonExisting { account_proof, .. } => account_proof,
        }
    }

    pub fn get_storage_proofs(&self) -> &HashMap<H256, Vec<NodeRLP>> {
        match self {
            Account::Existing { storage_proofs, .. } => storage_proofs,
            Account::NonExisting { storage_proofs, .. } => storage_proofs,
        }
    }
}

pub async fn get_latest_block_number(rpc_url: &str) -> eyre::Result<usize> {
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_blockNumber",
        "params": []
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;

    let res = response.json::<serde_json::Value>().await?;
    let mut bytes = decode_hex(get_result(res)?)?;
    bytes.reverse();
    bytes.resize(8, 0);
    let latest_bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| eyre::Error::msg("decode error".to_string()))?;

    Ok(usize::from_le_bytes(latest_bytes))
}

pub async fn get_block(rpc_url: &str, block_number: usize) -> eyre::Result<Block> {
    let block_number = format!("0x{block_number:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_getRawBlock",
        "params": [block_number]
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;

    let res = response.json::<serde_json::Value>().await?;
    let encoded_block = decode_hex(get_result(res)?)?;
    let block = Block::decode_unfinished(&encoded_block)?;
    Ok(block.0)
}

pub async fn get_account(
    rpc_url: &str,
    block_number: usize,
    address: &Address,
    storage_keys: &[H256],
) -> eyre::Result<Account> {
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

    let response = CLIENT.post(rpc_url).json(request).send().await?;

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
    } = get_result(response.json::<serde_json::Value>().await?)?;

    let account_proof = account_proof
        .into_iter()
        .map(decode_hex)
        .collect::<eyre::Result<Vec<_>>>()?;

    let (storage, storage_proofs) = storage_proof
        .into_iter()
        .map(|proof| -> eyre::Result<_> {
            let key: H256 = proof.key.parse()?;
            let value: U256 = proof.value.parse()?;
            let proofs = proof
                .proof
                .into_iter()
                .map(decode_hex)
                .collect::<eyre::Result<Vec<_>, _>>()?;
            Ok(((key, value), (key, proofs)))
        })
        .collect::<eyre::Result<(HashMap<_, _>, HashMap<_, _>)>>()?;

    let root = account_proof
        .first()
        .ok_or(eyre::Error::msg("account proof is empty".to_string()))?;
    let other: Vec<_> = account_proof.iter().skip(1).cloned().collect();
    let trie = Trie::from_nodes(Some(root), &other)?;
    if trie.get(&hash_address(address))?.is_none() {
        return Ok(Account::NonExisting {
            account_proof,
            storage_proofs,
        });
    }

    let account_state = AccountState {
        nonce: u64::from_str_radix(nonce.trim_start_matches("0x"), 16)?,
        balance: balance.parse()?,
        storage_root: storage_hash.parse()?,
        code_hash: code_hash.parse()?,
    };

    let code = if account_state.code_hash != *EMPTY_KECCACK_HASH {
        Some(get_code(rpc_url, block_number, address).await?)
    } else {
        None
    };

    Ok(Account::Existing {
        account_state,
        storage,
        account_proof,
        storage_proofs,
        code,
    })
}

pub async fn retry<F, I>(mut fut: F) -> eyre::Result<I>
where
    F: Task<Item = I, Error = eyre::Report>,
{
    let policy = RetryPolicy::exponential(Duration::from_secs(1)).with_jitter(true);
    policy.retry(|| fut.call()).await
}

async fn get_code(rpc_url: &str, block_number: usize, address: &Address) -> eyre::Result<Bytes> {
    let block_number = format!("0x{block_number:x}");
    let address = format!("0x{address:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getCode",
        "params": [address, block_number]
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;

    let res = response.json::<serde_json::Value>().await?;
    let owner_bytes = decode_hex(get_result(res)?)?;
    Ok(Bytes::from_owner(owner_bytes))
}

fn get_result<T: DeserializeOwned>(response: serde_json::Value) -> eyre::Result<T> {
    match response.get("result") {
        Some(result) => Ok(serde_json::from_value(result.clone())?),
        None => Err(eyre::Error::msg(format!(
            "result not found, response is: {response}"
        ))),
    }
}

fn decode_hex(hex: String) -> eyre::Result<Vec<u8>> {
    let mut trimmed = hex.trim_start_matches("0x").to_string();
    if trimmed.len() % 2 != 0 {
        trimmed = "0".to_string() + &trimmed;
    }
    Ok(hex::decode(trimmed)?)
}

pub async fn get_tx_block(tx: &str, rpc_url: &str) -> eyre::Result<usize> {
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getTransactionByHash",
        "params": [tx]
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;

    let res = response.json::<serde_json::Value>().await?;
    let res = res.get("result").ok_or(eyre::Error::msg("result key"))?;
    let block_number = res
        .get("blockNumber")
        .and_then(|v| v.as_str())
        .ok_or(eyre::Error::msg("bad blockNumber key"))?;
    let block_number = usize::from_str_radix(block_number.trim_start_matches("0x"), 16)?;
    Ok(block_number)
}

#[cfg(test)]
mod test {
    use super::*;

    const RPC_URL: &str = "<to-complete>";
    const VITALIK_ADDR: &str = "d8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

    #[ignore = "needs to manually set rpc url in constant"]
    #[tokio::test]
    async fn get_block_works() {
        let block_number = get_latest_block_number(RPC_URL).await.unwrap();
        get_block(RPC_URL, block_number).await.unwrap();
    }

    #[ignore = "needs to manually set rpc url in constant"]
    #[tokio::test]
    async fn get_account_works() {
        let block_number = get_latest_block_number(RPC_URL).await.unwrap();
        get_account(
            RPC_URL,
            block_number,
            &Address::from_slice(&hex::decode(VITALIK_ADDR).unwrap()),
            &[],
        )
        .await
        .unwrap();
    }
}
