lazy_static::lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use ethrex_common::types::{AccountState, EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH};
use ethrex_common::{serde_utils, Address};
use ethrex_common::{types::BlockNumber, BigEndianHash, Bytes, H256, U256};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::clients::auth::RpcResponse;
use ethrex_rpc::types::block::RpcBlock;
use ethrex_storage::Store;
use keccak_hash::keccak;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinSet;
use tracing::info;

const MAX_ACCOUNTS: usize = 256;

#[derive(Deserialize, Debug)]
struct Dump {
    #[serde(rename = "root")]
    state_root: H256,
    #[serde(deserialize_with = "deser_account_dump_map")]
    accounts: BTreeMap<H256, DumpAccount>,
    #[serde(default)]
    next: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DumpAccount {
    #[serde(deserialize_with = "serde_utils::u256::deser_dec_str")]
    balance: U256,
    nonce: u64,
    #[serde(rename = "root")]
    storage_root: H256,
    code_hash: H256,
    #[serde(default, with = "serde_utils::bytes")]
    code: Bytes,
    #[serde(default)]
    // Didn't parse a dump with storage yet, may fail
    storage: HashMap<H256, U256>,
    address: Option<Address>,
    #[serde(rename = "key")]
    hashed_address: Option<H256>,
}

pub async fn archive_sync(
    archive_ipc_path: &str,
    block_number: BlockNumber,
    store: Store,
) -> eyre::Result<()> {
    let mut stream = UnixStream::connect(archive_ipc_path).await?;
    let mut start = H256::zero();
    let mut state_trie_root = *EMPTY_TRIE_HASH;
    let mut should_continue = true;
    let mut state_root = None;
    while should_continue {
        let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_accountRange",
        "params": [format!("{block_number:#x}"), format!("{start:#x}"), MAX_ACCOUNTS, false, false, false]
        });
        let response = send_ipc_json_request(&mut stream, request).await?;
        let dump: Dump = serde_json::from_value(response)?;
        // Sanity check
        if *state_root.get_or_insert(dump.state_root) != dump.state_root {
            return Err(eyre::ErrReport::msg(
                "Archive node yieled different state roots for the same block dump",
            ));
        }
        should_continue = dump.next.is_some();
        if should_continue {
            start = hash_next(*dump.accounts.last_key_value().unwrap().0);
        }
        // Process dump
        let instant = Instant::now();
        state_trie_root = process_dump(dump, store.clone(), state_trie_root).await?;
        info!(
            "Processed Dump of {MAX_ACCOUNTS} accounts in {} ms",
            instant.elapsed().as_millis()
        );
    }
    // Request block so we can store it and mark it as canonical
    let request = &json!({
    "id": 1,
    "jsonrpc": "2.0",
    "method": "eth_getBlockByNumber",
    "params": [format!("{block_number:#x}"), true]
    });
    let response = send_ipc_json_request(&mut stream, request).await?;
    let rpc_block: RpcBlock = serde_json::from_value(response)?;
    let block = rpc_block.into_full_block().ok_or(eyre::ErrReport::msg(
        "Requested block with full transactions but only obtained hashes",
    ))?;
    if state_trie_root != block.header.state_root {
        return Err(eyre::ErrReport::msg(
            "State root doesn't match the one in the header after archive sync",
        ));
    }
    let block_number = block.header.number;
    let block_hash = block.hash();
    store.add_block(block).await?;
    store.set_canonical_block(block_number, block_hash).await?;
    store.update_latest_block_number(block_number).await?;

    Ok(())
}

/// Adds all dump accounts to the trie on top of the current root, returns the next root
/// This could be improved in the future to use an in_memory trie with async db writes
async fn process_dump(dump: Dump, store: Store, current_root: H256) -> eyre::Result<H256> {
    let mut storage_tasks = JoinSet::new();
    let mut state_trie = store.open_state_trie(current_root)?;
    for (hashed_address, dump_account) in dump.accounts.into_iter() {
        // Add account to state trie
        // Maybe we can validate the dump account here? or while deserializing
        state_trie.insert(
            hashed_address.0.to_vec(),
            dump_account.get_account_state().encode_to_vec(),
        )?;
        // Add code to DB if it is not empty
        if dump_account.code_hash != *EMPTY_KECCACK_HASH {
            store
                .add_account_code(dump_account.code_hash, dump_account.code.clone())
                .await?;
        }
        // Process storage trie if it is not empty
        if dump_account.storage_root != *EMPTY_TRIE_HASH {
            storage_tasks.spawn(process_dump_storage(
                dump_account.storage,
                store.clone(),
                hashed_address,
                dump_account.storage_root,
            ));
        }
    }
    for res in storage_tasks.join_all().await {
        res?;
    }
    Ok(state_trie.hash()?)
}

async fn process_dump_storage(
    dump_storage: HashMap<H256, U256>,
    store: Store,
    hashed_address: H256,
    storage_root: H256,
) -> eyre::Result<()> {
    let mut trie = store.open_storage_trie(hashed_address, *EMPTY_TRIE_HASH)?;
    for (key, val) in dump_storage {
        trie.insert(key.0.to_vec(), val.encode_to_vec())?;
    }
    if trie.hash()? != storage_root {
        Err(eyre::ErrReport::msg(
            "Storage root doesn't match the one in the account during archive sync",
        ))
    } else {
        Ok(())
    }
}

async fn send_ipc_json_request(stream: &mut UnixStream, request: &Value) -> eyre::Result<Value> {
    stream.write_all(request.to_string().as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    let mut response = Vec::new();
    while stream.read_buf(&mut response).await? != 0 {
        if response.ends_with(b"\n") {
            break;
        }
    }
    let response: RpcResponse = serde_json::from_slice(&response)?;
    match response {
        RpcResponse::Success(success_res) => Ok(success_res.result),
        RpcResponse::Error(error_res) => Err(eyre::ErrReport::msg(error_res.error.message)),
    }
}

fn hash_next(hash: H256) -> H256 {
    H256::from_uint(&(hash.into_uint() + 1))
}

/// Deserializes a map of Address -> DumpAccount into a sorted map of HashedAddress -> DumpAccount
/// This is necessary as `debug_getAccountRange` sorts accounts by hashed address
fn deser_account_dump_map<'de, D>(d: D) -> Result<BTreeMap<H256, DumpAccount>, D::Error>
where
    D: Deserializer<'de>,
{
    let map = HashMap::<Address, DumpAccount>::deserialize(d)?;
    // Order dump accounts by hashed address
    map.into_iter()
        .map(|(addr, acc)| {
            // Sanity check
            if acc.address.is_some_and(|acc_addr| acc_addr != addr) {
                Err(serde::de::Error::custom(
                    "DumpAccount address field doesn't match it's key in the Dump".to_string(),
                ))
            } else {
                let hashed_addr = acc.hashed_address.unwrap_or_else(|| keccak(addr));
                Ok((hashed_addr, acc))
            }
        })
        .collect()
}

impl DumpAccount {
    fn get_account_state(&self) -> AccountState {
        AccountState {
            nonce: self.nonce,
            balance: self.balance,
            storage_root: self.storage_root,
            code_hash: self.code_hash,
        }
    }
}
