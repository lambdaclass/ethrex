use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use again::{RetryPolicy, Task};

use bytes::Bytes;
use ethrex_common::{Address, H256, U256, constants::EMPTY_KECCACK_HASH, types::AccountState};
use ethrex_rpc::types::block::RpcBlock;
use ethrex_storage::hash_address;
use ethrex_trie::Trie;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use lazy_static::lazy_static;
use sha3::Digest;

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

pub async fn get_block(
    rpc_url: &str,
    block_number: usize,
    hydrated: bool,
) -> eyre::Result<RpcBlock> {
    let block_number = format!("0x{block_number:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getBlockByNumber",
        "params": [block_number, hydrated]
    });
    let response = CLIENT.post(rpc_url).json(request).send().await?;
    let rpc_block: RpcBlock = get_result(response.json::<serde_json::Value>().await?)?;
    Ok(rpc_block)
}

pub async fn get_account(
    rpc_url: &str,
    block_number: usize,
    address: &Address,
    storage_keys: &[H256],
    codes: &Arc<Mutex<HashMap<H256, Bytes>>>,
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

    let mut state_nodes = BTreeMap::new();
    for node in &account_proof {
        let hash = sha3::Keccak256::digest(node);
        state_nodes.insert(H256::from_slice(&hash), node.clone());
    }

    let hash = H256::from_slice(&sha3::Keccak256::digest(root));
    let trie = Trie::from_nodes(hash, &state_nodes)?;
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
        if let Some(cached_code) = codes.lock().unwrap().get(&account_state.code_hash) {
            Some(cached_code.clone())
        } else {
            let fetched_code = get_code(rpc_url, block_number, address).await?;
            let mut codes_lock = codes.lock().unwrap();
            codes_lock.insert(account_state.code_hash, fetched_code.clone());
            Some(fetched_code)
        }
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
