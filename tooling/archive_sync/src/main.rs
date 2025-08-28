lazy_static::lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

use cita_trie::{MemoryDB as CitaMemoryDB, PatriciaTrie as CitaTrie, Trie as CitaTrieTrait};
use clap::{ArgGroup, Parser};
use ethrex::initializers::open_store;
use ethrex::utils::{default_datadir, init_datadir};
use ethrex_common::types::BlockHash;
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
use hasher::HasherKeccak;
use keccak_hash::keccak;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinSet;
use tracing::{debug, info};
use tracing_subscriber::FmtSubscriber;

/// Max account dumps to ask for in a single request. The current value matches geth's maximum output.
const MAX_ACCOUNTS: usize = 1;
/// Amount of blocks before the target block to request hashes for. These may be needed to execute the next block after the target block.
const BLOCK_HASH_LOOKUP_DEPTH: u64 = 128;

#[derive(Deserialize, Debug, Serialize)]
struct Dump {
    #[serde(rename = "root")]
    state_root: H256,
    accounts: HashMap<MaybeAddress, DumpAccount>,
    #[serde(default)]
    next: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
enum MaybeAddress {
    Address(Address),
    HashesAddress(H256),
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpAccount {
    #[serde(with = "serde_utils::u256::dec_str")]
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

use serde::de::Error;
use std::str::FromStr;

impl<'de> Deserialize<'de> for MaybeAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Check if it is an address or a pre(hashed_address)
        let str = String::deserialize(deserializer)?;
        if let Some(str) = str.strip_prefix("pre(") {
            Ok(MaybeAddress::HashesAddress(
                H256::from_str(str.trim_end_matches(")")).map_err(|err| D::Error::custom(err))?,
            ))
        } else {
            Ok(MaybeAddress::Address(
                Address::from_str(&str).map_err(|err| D::Error::custom(err))?,
            ))
        }
    }
}

impl Serialize for MaybeAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            MaybeAddress::Address(address) => address.serialize(serializer),
            MaybeAddress::HashesAddress(hash) => serializer.serialize_str(&format!("pre({hash})")),
        }
    }
}

impl MaybeAddress {
    fn to_hashed(self) -> H256 {
        match self {
            MaybeAddress::Address(address) => keccak(address),
            MaybeAddress::HashesAddress(hashed) => hashed,
        }
    }
}

fn cita_trie() -> CitaTrie<CitaMemoryDB, HasherKeccak> {
    let memdb = Arc::new(CitaMemoryDB::new(true));
    let hasher = Arc::new(HasherKeccak::new());

    CitaTrie::new(Arc::clone(&memdb), Arc::clone(&hasher))
}

pub async fn archive_sync(
    archive_ipc_path: Option<String>,
    block_number: BlockNumber,
    output_dir: Option<String>,
    input_dir: Option<String>,
    no_sync: bool,
    store: Store,
) -> eyre::Result<()> {
    let sync_start: Instant = Instant::now();
    let mut dump_reader = if let Some(ipc_path) = archive_ipc_path {
        DumpReader::new_from_ipc(&ipc_path, block_number).await?
    } else {
        DumpReader::new_from_dir(input_dir.unwrap())?
    };
    let dump_writer = output_dir.map(DumpDirWriter::new).transpose()?;
    let mut dump_processor = if no_sync {
        DumpProcessor::new_no_sync(dump_writer)
    } else {
        DumpProcessor::new_sync(dump_writer, store)
    };
    let mut should_continue = true;
    // Fetch and process dumps until we have the full block state
    while should_continue {
        let dump = dump_reader.read_dump().await?;
        should_continue = dump_processor.process_dump(dump).await?;
    }
    // Fetch the block itself so we can mark it as canonical
    let rlp_block = dump_reader.read_rlp_block().await?;
    // Fetch the block hashes of the previous `BLOCK_HASH_LOOKUP_DEPTH` blocks
    // as we might need them to execute the next blocks after archive sync
    let block_hashes = dump_reader.read_block_hashes().await?;
    // Process both as part of a FCU
    dump_processor
        .process_rlp_block_and_block_hashes(rlp_block, block_hashes)
        .await?;
    let sync_time = mseconds_to_readable(sync_start.elapsed().as_millis());
    info!("Archive Sync complete in {sync_time}");
    Ok(())
}

/// Adds all dump accounts to the trie on top of the current root, returns the next root
/// This could be improved in the future to use an in_memory trie with async db writes
async fn process_dump(dump: Dump, store: Store, current_root: H256) -> eyre::Result<H256> {
    let mut storage_tasks = JoinSet::new();
    let mut state_trie = store.open_state_trie(current_root)?;
    for (address, dump_account) in dump.accounts.into_iter() {
        let hashed_address = dump_account
            .hashed_address
            .unwrap_or_else(|| address.to_hashed());
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
    info!("processing dump storage: {dump_storage:?}");
    let mut trie = store.open_storage_trie(hashed_address, *EMPTY_TRIE_HASH)?;
    let mut cita_trie = cita_trie();
    for (key, val) in dump_storage {
        info!("Adding key: {key}, value {val:#x}");
        // The key we receive is the preimage of the one stored in the trie
        let valu = val;
        let val_enc = val.encode_to_vec();
        let val_dec = U256::decode(&val_enc).unwrap();
        assert_eq!(valu, val_dec);
        trie.insert(keccak(key.0).0.to_vec(), val.encode_to_vec())?;
        cita_trie
            .insert(keccak(key.0).0.to_vec(), val.encode_to_vec())
            .unwrap();
    }
    let cita_hash = cita_trie.root().unwrap();
    if trie.hash()? != storage_root {
        info!(
            "storage hash mismatch: calced {} vs received {} vs cita {}",
            trie.hash()?,
            storage_root,
            H256::from_slice(&cita_hash)
        );
        // Err(eyre::ErrReport::msg(
        //     "Storage root doesn't match the one in the account during archive sync",
        // ))
        Ok(())
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

/// Struct in charge of processing incoming state data
/// Depending on its optional fields processing can refer to either writing the state into files
/// and/or rebuilding the block's state in the DB
struct DumpProcessor {
    state_root: Option<H256>,
    // Current Trie Root + Store. Set to None if state sync is disabled
    sync_state: Option<(H256, Store)>,
    writer: Option<DumpDirWriter>,
}

impl DumpProcessor {
    /// Create a new DumpProcessor that will rebuild a Block's state based on incoming state dumps
    /// And which may write incoming data into files if writer is set
    fn new_sync(writer: Option<DumpDirWriter>, store: Store) -> Self {
        Self {
            state_root: None,
            sync_state: Some((*EMPTY_TRIE_HASH, store)),
            writer,
        }
    }

    /// Create a new DumpProcessor which may write incoming data into files if writer is set
    fn new_no_sync(writer: Option<DumpDirWriter>) -> Self {
        Self {
            state_root: None,
            sync_state: None,
            writer,
        }
    }

    /// Process incoming state dump by either writing it to a file and/or using it to rebuild the partial state
    /// Will fail if the incoming dump's state root differs from the previously processed dump
    async fn process_dump(&mut self, dump: Dump) -> eyre::Result<bool> {
        // Sanity check
        if *self.state_root.get_or_insert(dump.state_root) != dump.state_root {
            return Err(eyre::ErrReport::msg(
                "Archive node yielded different state roots for the same block dump",
            ));
        }
        let should_continue = dump.next.is_some();
        // Write dump if we have an output
        if let Some(writer) = self.writer.as_mut() {
            writer.write_dump(&dump)?;
        }
        // Process dump
        if let Some((current_root, store)) = self.sync_state.as_mut() {
            let instant = Instant::now();
            *current_root = process_dump(dump, store.clone(), *current_root).await?;
            info!(
                "Processed Dump of {MAX_ACCOUNTS} accounts in {}",
                mseconds_to_readable(instant.elapsed().as_millis())
            );
        }
        Ok(should_continue)
    }

    /// Process the incoming RLP-encoded Block by either writing it to a file and/or adding it as head of the canonical chain.
    /// In the later case, the rebuilt state root will be chacked againts the block's state root
    /// Processes the incoming list of block hashes by either writing them to a file and/or marking
    /// them as part of the canonical chain. This will be necessary in order to execute blocks after the target block
    async fn process_rlp_block_and_block_hashes(
        &mut self,
        rlp_block: Vec<u8>,
        block_hashes: Vec<(BlockNumber, BlockHash)>,
    ) -> eyre::Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.write_rlp_block(&rlp_block)?;
            writer.write_hashes_file(&block_hashes)?;
        }
        if let Some((current_root, store)) = self.sync_state.as_ref() {
            let block = Block::decode(&rlp_block)?;
            let block_number = block.header.number;
            let block_hash = block.hash();

            if *current_root != block.header.state_root {
                return Err(eyre::ErrReport::msg(
                    "State root doesn't match the one in the header after archive sync",
                ));
            }

            store.add_block(block).await?;
            store
                .forkchoice_update(Some(block_hashes), block_number, block_hash, None, None)
                .await?;
            info!("Head of local chain is now block {block_number} with hash {block_hash}");
        }
        Ok(())
    }
}

/// Struct in charge of writing state data into files on a given directory
struct DumpDirWriter {
    dirname: String,
    current_file: usize,
}

impl DumpDirWriter {
    /// Create a new DumpDirWriter which will write state data to files the given directory
    /// It will create the directory if it doesn't exist yet
    fn new(dirname: String) -> eyre::Result<DumpDirWriter> {
        if !std::path::Path::new(&dirname).exists() {
            std::fs::create_dir(&dirname)?;
        }
        Ok(Self {
            dirname,
            current_file: 0,
        })
    }

    /// Writes the incoming dump into a json file named `dump_n.json` at the directory set
    /// in the struct's creation. Where n represents the order at which each dump was received
    fn write_dump(&mut self, dump: &Dump) -> eyre::Result<()> {
        let dump_file = File::create(
            std::path::Path::new(&self.dirname).join(format!("dump_{}.json", self.current_file)),
        )?;
        serde_json::to_writer(dump_file, dump)?;
        self.current_file += 1;
        Ok(())
    }

    /// Writes the incoming RLP-encoded block into file named `block.rlp`
    /// at the directory set in the struct's creation
    fn write_rlp_block(&mut self, rlp: &[u8]) -> eyre::Result<()> {
        let mut block_file: File =
            File::create(std::path::Path::new(&self.dirname).join("block.rlp"))?;
        block_file.write_all(rlp)?;
        Ok(())
    }

    /// Writes the incoming block hahses into a json file named `block_hashes.json`
    /// at the directory set in the struct's creation
    fn write_hashes_file(
        &mut self,
        block_hashes: &Vec<(BlockNumber, BlockHash)>,
    ) -> eyre::Result<()> {
        let block_hashes_file =
            File::create(std::path::Path::new(&self.dirname).join("block_hashes.json"))?;
        serde_json::to_writer(block_hashes_file, block_hashes)?;
        Ok(())
    }
}

/// Struct in charge of fetching state data
/// This data may come from either IPC comunication with an active archive node
/// or a directory of files obtained from a previous archive-sync execution using --output-dir flag
enum DumpReader {
    Dir(DumpDirReader),
    Ipc(DumpIpcReader),
}

/// Struct in charge of reading state data from a directory of files obtained
/// from a previous archive-sync execution using --output-dir flag
struct DumpDirReader {
    dirname: String,
    current_file: usize,
}

/// Struct in charge of fetching state data from an IPC connection with an active archive node
struct DumpIpcReader {
    stream: UnixStream,
    block_number: BlockNumber,
    start: H256,
}

impl DumpReader {
    /// Create a new DumpReader that will read state data from the given directory
    fn new_from_dir(dirname: String) -> eyre::Result<Self> {
        Ok(Self::Dir(DumpDirReader::new(dirname)?))
    }

    /// Create a new DumpReader that will read state data from an archive node given the path to its IPC file
    async fn new_from_ipc(archive_ipc_path: &str, block_number: BlockNumber) -> eyre::Result<Self> {
        Ok(Self::Ipc(
            DumpIpcReader::new(archive_ipc_path, block_number).await?,
        ))
    }

    /// Read the next state dump, either from a file or from an active IPC connection
    async fn read_dump(&mut self) -> eyre::Result<Dump> {
        match self {
            DumpReader::Dir(dump_dir_reader) => dump_dir_reader.read_dump(),
            DumpReader::Ipc(dump_ipc_reader) => dump_ipc_reader.read_dump().await,
        }
    }

    /// Read the target RLP-encoded block, either from a file or from an active IPC connection
    async fn read_rlp_block(&mut self) -> eyre::Result<Vec<u8>> {
        match self {
            DumpReader::Dir(dump_dir_reader) => dump_dir_reader.read_rlp_block(),
            DumpReader::Ipc(dump_ipc_reader) => dump_ipc_reader.read_rlp_block().await,
        }
    }

    /// Read hashes of the `BLOCK_HASH_LOOKUP_DEPTH` blocks before the target block,
    ///  either from a file or from an active IPC connection
    async fn read_block_hashes(&mut self) -> eyre::Result<Vec<(BlockNumber, BlockHash)>> {
        match self {
            DumpReader::Dir(dump_dir_reader) => dump_dir_reader.read_block_hashes(),
            DumpReader::Ipc(dump_ipc_reader) => dump_ipc_reader.read_block_hashes().await,
        }
    }
}

impl DumpDirReader {
    /// Create a new DumpDirReader that will read state data from the given directory
    fn new(dirname: String) -> eyre::Result<DumpDirReader> {
        if !std::path::Path::new(&dirname).exists() {
            return Err(eyre::Error::msg("Input directory doesn't exist"));
        }
        Ok(Self {
            dirname,
            current_file: 0,
        })
    }

    /// Read the next dump file from the directory set at creation
    fn read_dump(&mut self) -> eyre::Result<Dump> {
        let dump_file = File::open(
            std::path::Path::new(&self.dirname).join(format!("dump_{}.json", self.current_file)),
        )?;
        self.current_file += 1;
        Ok(serde_json::from_reader(dump_file)?)
    }

    /// Read the rlp block file from the directory set at creation
    fn read_rlp_block(&mut self) -> eyre::Result<Vec<u8>> {
        let mut block_file = File::open(std::path::Path::new(&self.dirname).join("block.rlp"))?;
        let mut buffer = Vec::<u8>::new();
        block_file.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    /// Read the block hashes file from the directory set at creation
    fn read_block_hashes(&mut self) -> eyre::Result<Vec<(BlockNumber, BlockHash)>> {
        let hashes_file =
            File::open(std::path::Path::new(&self.dirname).join("block_hashes.json"))?;
        Ok(serde_json::from_reader(hashes_file)?)
    }
}

impl DumpIpcReader {
    /// Create a new DumpIpcReader that will fetch incoming data by connecting to an active archive node
    /// given the path to its IPC file
    async fn new(archive_ipc_path: &str, block_number: BlockNumber) -> eyre::Result<DumpIpcReader> {
        let stream = UnixStream::connect(archive_ipc_path).await?;
        Ok(Self {
            stream,
            block_number,
            start: H256::zero(),
        })
    }

    /// Fetches the nex state dump from the archive node it is currently connected to via IPC
    async fn read_dump(&mut self) -> eyre::Result<Dump> {
        // [debug_accountRange](https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-debug#debugaccountrange)
        let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_accountRange",
        "params": [format!("{:#x}", self.block_number), format!("{:#x}", self.start), MAX_ACCOUNTS, false, false, true]
        });
        let response = send_ipc_json_request(&mut self.stream, request).await?;
        info!("IPC res: {response}");
        let dump: Dump = serde_json::from_value(response)?;
        // Find the next hash
        let last_key = dump
            .accounts
            .iter()
            .map(|(addr, acc)| {
                acc.hashed_address
                    .unwrap_or_else(|| addr.clone().to_hashed())
            })
            .max()
            .unwrap_or_default();
        self.start = hash_next(last_key);
        Ok(dump)
    }

    /// Fetches the RLP-encoded target blocks from the archive node it is currently connected to via IPC
    async fn read_rlp_block(&mut self) -> eyre::Result<Vec<u8>> {
        // Request block so we can store it and mark it as canonical
        let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_getRawBlock",
        "params": [format!("{:#x}", self.block_number)]
        });
        let response = send_ipc_json_request(&mut self.stream, request).await?;
        let rlp_block_str: String = serde_json::from_value(response)?;
        let rlp_block = hex::decode(rlp_block_str.trim_start_matches("0x"))?;
        Ok(rlp_block)
    }

    /// Fetch the block hashes for the `BLOCK_HASH_LOOKUP_DEPTH` blocks before the current one
    /// from the archive node it is currently connected to via IPC
    async fn read_block_hashes(&mut self) -> eyre::Result<Vec<(BlockNumber, BlockHash)>> {
        let mut res = Vec::new();
        for offset in 1..BLOCK_HASH_LOOKUP_DEPTH {
            let Some(block_number) = self.block_number.checked_sub(offset) else {
                break;
            };
            let request = &json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "debug_dbAncient",
            "params": ["hashes", block_number]
            });
            let response = send_ipc_json_request(&mut self.stream, request).await?;
            let block_hash: BlockHash = serde_json::from_value(response)?;
            res.push((block_number, block_hash));
        }
        Ok(res)
    }
}

#[derive(Parser)]
#[clap(group = ArgGroup::new("input").required(true).args(&["ipc_path", "input_dir"]).multiple(false))]
struct Args {
    #[arg(
        required = true,
        value_name = "NUMBER",
        help = "Block number to sync to"
    )]
    block_number: BlockNumber,
    #[arg(
        long = "datadir",
        value_name = "DATABASE_DIRECTORY",
        default_value_t = default_datadir(),
        help = "Receives the name of the directory where the Database is located.",
        long_help = "If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.",
        env = "ETHREX_DATADIR"
    )]
    pub datadir: String,
    #[arg(
        long = "ipc_path",
        value_name = "IPC_PATH",
        help = "Path to the ipc of the archive node."
    )]
    ipc_path: Option<String>,
    #[arg(
        long = "input_dir",
        value_name = "INPUT_DIRECTORY",
        help = "Receives the name of the directory where the State Dump will be read from."
    )]
    pub input_dir: Option<String>,
    #[arg(
        long = "output_dir",
        value_name = "OUTPUT_DIRECTORY",
        help = "Receives the name of the directory where the State Dump will be written to."
    )]
    pub output_dir: Option<String>,
    #[arg(
        long = "no_sync",
        value_name = "NO_SYNC",
        help = "If enabled, the node will not process the incoming state. Only usable if --output_dir is set",
        requires = "output_dir"
    )]
    pub no_sync: bool,
}

#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    let args = Args::parse();
    tracing::subscriber::set_global_default(FmtSubscriber::new())
        .expect("setting default subscriber failed");
    let data_dir = init_datadir(&args.datadir);
    let store = open_store(&data_dir);
    archive_sync(
        args.ipc_path,
        args.block_number,
        args.output_dir,
        args.input_dir,
        args.no_sync,
        store,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test() {
        tracing::subscriber::set_global_default(FmtSubscriber::new())
            .expect("setting default subscriber failed");
        let ipc_input = r#"{"root":"3932ca77e85e06d8524fd2d456a427ef4938b6cd18c98d3a6159afe911c8396c","accounts":{"0xe3b30DE5Be5B32A1316D0686BBE54BC7F0FeF23c":{"balance":"0","nonce":1,"root":"0xbbad49c3e342b927f5ac7c041cfcf93b2910d2e6725428ba067952f896a4ef7e","codeHash":"0x337c29fd9976d67b66b28034c1414c04861ce13b19a267c6e01d66f2cdb6bfba","code":"0x60606040525b603c5b60006010603e565b9050593681016040523660008237602060003683856040603f5a0204f41560545760206000f35bfe5b50565b005b73c3b2ae46792547a96b9f84405e36d0e07edcd05c5b905600a165627a7a7230582062a884f947232ada573f95940cce9c8bfb7e4e14e21df5af4e884941afb55e590029","storage":{"0x0000000000000000000000000000000000000000000000000000000000000001":"7a30f7736e48d6599356464ba4c150d8da0302ff","0x0000000000000000000000000000000000000000000000000000000000000002":"3da25693590cc69abc4db528c9100274e47677df","0x0000000000000000000000000000000000000000000000000000000000000003":"01"},"address":"0xe3b30de5be5b32a1316d0686bbe54bc7f0fef23c","key":"0x00027f1d9e19e9285dad2dc6b3b37dfd7dfe5a25dfb1ce4521d4c1220e03bef4"}},"next":"AAJ/LjgV+1RZWRkAWfopljIQafWGPo6xdpDcKvs6biI="}"#;
        let dump: Dump = serde_json::from_str(&ipc_input).unwrap();
        let store = Store::new("uwu.memory", ethrex_storage::EngineType::InMemory).unwrap();
        process_dump(dump, store, *EMPTY_TRIE_HASH).await.unwrap();
    }
}

/*

    IPC res: {"root":"3932ca77e85e06d8524fd2d456a427ef4938b6cd18c98d3a6159afe911c8396c","accounts":{"0xe3b30DE5Be5B32A1316D0686BBE54BC7F0FeF23c":{"balance":"0","nonce":1,"root":"0xbbad49c3e342b927f5ac7c041cfcf93b2910d2e6725428ba067952f896a4ef7e","codeHash":"0x337c29fd9976d67b66b28034c1414c04861ce13b19a267c6e01d66f2cdb6bfba","code":"0x60606040525b603c5b60006010603e565b9050593681016040523660008237602060003683856040603f5a0204f41560545760206000f35bfe5b50565b005b73c3b2ae46792547a96b9f84405e36d0e07edcd05c5b905600a165627a7a7230582062a884f947232ada573f95940cce9c8bfb7e4e14e21df5af4e884941afb55e590029",
    "storage":{
    "0x0000000000000000000000000000000000000000000000000000000000000001":"7a30f7736e48d6599356464ba4c150d8da0302ff"
    "0x0000000000000000000000000000000000000000000000000000000000000002":"3da25693590cc69abc4db528c9100274e47677df"
    "0x0000000000000000000000000000000000000000000000000000000000000003":"01"},
    "address":"0xe3b30de5be5b32a1316d0686bbe54bc7f0fef23c","key":"0x00027f1d9e19e9285dad2dc6b3b37dfd7dfe5a25dfb1ce4521d4c1220e03bef4"}},"next":"AAJ/LjgV+1RZWRkAWfopljIQafWGPo6xdpDcKvs6biI="}
    processing dump storage: {0x0000000000000000000000000000000000000000000000000000000000000003: 1, 0x0000000000000000000000000000000000000000000000000000000000000001: 697588865823728504998797776820623519631662056191, 0x0000000000000000000000000000000000000000000000000000000000000002: 351868699538881868049991907159917927825854068703}
    Adding key: 0x0000…0003, value 0x1
    Adding key: 0x0000…0001, value 0x7a30f7736e48d6599356464ba4c150d8da0302ff
    Adding key: 0x0000…0002, value 0x3da25693590cc69abc4db528c9100274e47677df
    storage hash mismatch: calced 0xd34d…8df0 vs received 0xbbad…ef7e vs cita 0xd34d…8df0
*/

/*
IPC res: {"root":"3932ca77e85e06d8524fd2d456a427ef4938b6cd18c98d3a6159afe911c8396c","accounts":{"0xEA46927B4Fc92248d052299FBFCC6778421930C6":{"balance":"7931794000000000","nonce":1,"root":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","codeHash":"0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470","address":"0xea46927b4fc92248d052299fbfcc6778421930c6","key":"0x00000013653234c2d78dcdc645c5141e358ef2e590fe5278778ba729ff5ffd95"}},"next":"AAAAS07Z5IKDj98CtF3DFcBGhanjeEjmBlrpuAWSUZA="}

IPC res: {"root":"3932ca77e85e06d8524fd2d456a427ef4938b6cd18c98d3a6159afe911c8396c","accounts":{"pre(0x0000004b4ed9e482838fdf02b45dc315c04685a9e37848e6065ae9b805925190)":{"balance":"0","nonce":1,"root":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","codeHash":"0x562d59a51820d47f520c975e0b2bcffac644a509749a3161f481f57b6e826d21","code":"0x363d3d373d3d3d363d730de8bf93da2f7eecb3d9169422413a9bef4ef6285af43d82803e903d91602b57fd5bf3","key":"0x0000004b4ed9e482838fdf02b45dc315c04685a9e37848e6065ae9b805925190"}},"next":"AAAAjDjXaddcGtHeZmDaUe3BA5TBHFD/mgyp6LizXcI="} */
