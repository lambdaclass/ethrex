lazy_static::lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

use clap::Parser;
use ethrex::DEFAULT_DATADIR;
use ethrex::initializers::open_store;
use ethrex::utils::set_datadir;
use ethrex_common::{Address, serde_utils};
use ethrex_common::{BigEndianHash, Bytes, H256, U256, types::BlockNumber};
use ethrex_common::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, Block},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::clients::auth::RpcResponse;
use ethrex_storage::Store;
use keccak_hash::keccak;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinSet;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

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
    let sync_start = Instant::now();
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
            "Processed Dump of {MAX_ACCOUNTS} accounts in {}",
            mseconds_to_readable(instant.elapsed().as_millis())
        );
    }
    // Request block so we can store it and mark it as canonical
    let request = &json!({
    "id": 1,
    "jsonrpc": "2.0",
    "method": "debug_getRawBlock",
    "params": [format!("{block_number:#x}")]
    });
    let response = send_ipc_json_request(&mut stream, request).await?;
    let rlp_block_str: String = serde_json::from_value(response)?;
    let rlp_block = hex::decode(rlp_block_str.trim_start_matches("0x"))?;
    let block = Block::decode(&rlp_block)?;
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
    let sync_time = mseconds_to_readable(sync_start.elapsed().as_millis());
    info!(
        "Archive Sync complete in {sync_time}.\nHead of local chain is now block {block_number} with hash {block_hash}"
    );
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
        // The key we receive is the preimage of the one stored in the trie
        trie.insert(keccak(key.0).0.to_vec(), val.encode_to_vec())?;
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

fn mseconds_to_readable(mut mseconds: u128) -> String {
    const DAY: u128 = 24 * HOUR;
    const HOUR: u128 = 60 * MINUTE;
    const MINUTE: u128 = 60 * SECOND;
    const SECOND: u128 = 1000 * MSECOND;
    const MSECOND: u128 = 1;
    let mut res = String::new();
    let mut apply_time_unit = |unit_in_ms: u128, unit_str: &str| {
        if mseconds > unit_in_ms {
            let amount_of_unit = mseconds / unit_in_ms;
            res.push_str(&format!("{amount_of_unit}{unit_str}"));
            mseconds -= unit_in_ms * amount_of_unit
        }
    };
    apply_time_unit(DAY, "d");
    apply_time_unit(HOUR, "h");
    apply_time_unit(MINUTE, "m");
    apply_time_unit(SECOND, "s");
    apply_time_unit(MSECOND, "ms");

    res
}

#[derive(Parser)]
struct Args {
    #[arg(
        required = true,
        value_name = "IPC_PATH",
        help = "Path to the ipc of the archive node."
    )]
    archive_node_ipc: String,
    #[arg(
        required = true,
        value_name = "NUMBER",
        help = "Block number to sync to"
    )]
    block_number: BlockNumber,
    #[arg(
        long = "datadir",
        value_name = "DATABASE_DIRECTORY",
        help = "If the datadir is the word `memory`, ethrex will use the InMemory Engine",
        default_value = DEFAULT_DATADIR,
        help = "Receives the name of the directory where the Database is located.",
        long_help = "If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.",
        help_heading = "Node options",
        env = "ETHREX_DATADIR"
    )]
    pub datadir: String,
}

#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    let args = Args::parse();
    tracing::subscriber::set_global_default(FmtSubscriber::new())
        .expect("setting default subscriber failed");
    let data_dir = set_datadir(&args.datadir);
    let store = open_store(&data_dir);
    archive_sync(&args.archive_node_ipc, args.block_number, store).await
}
