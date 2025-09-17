lazy_static::lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

use clap::{ArgGroup, Parser};
use ethrex::initializers::open_store;
use ethrex::utils::{default_datadir, init_datadir};
use ethrex_common::types::BlockHash;
use ethrex_common::utils::keccak;
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
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinSet;
use tracing::{debug, info};
use tracing_subscriber::FmtSubscriber;

/// Max account dumps to ask for in a single request. The current value matches geth's maximum output.
const MAX_ACCOUNTS: usize = 256;
/// Amount of blocks before the target block to request hashes for. These may be needed to execute the next block after the target block.
const BLOCK_HASH_LOOKUP_DEPTH: u64 = 128;
/// Amount of state dumps to process before updating checkpoint
const DUMPS_BEFORE_CHECKPOINT: usize = 10;

#[derive(Deserialize, Debug, Serialize)]
struct Dump {
    #[serde(rename = "root")]
    state_root: H256,
    accounts: HashMap<Address, DumpAccount>,
    #[serde(default)]
    next: Option<String>,
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

pub async fn archive_sync(
    archive_ipc_path: Option<String>,
    block_number: BlockNumber,
    output_dir: Option<String>,
    input_dir: Option<String>,
    no_sync: bool,
    checkpoint: Option<String>,
    store: Store,
) -> eyre::Result<()> {
    let sync_start: Instant = Instant::now();
    // Load checkpoint (if we have one)
    let prev_checkpoint = load_checkpoint(
        &checkpoint,
        archive_ipc_path.is_some(),
        input_dir.is_some(),
        output_dir.is_some(),
        no_sync,
    )?;
    let mut dump_reader = if let Some(ipc_path) = archive_ipc_path {
        DumpReader::new_from_ipc(&ipc_path, block_number, &prev_checkpoint).await?
    } else {
        DumpReader::new_from_dir(input_dir.unwrap(), &prev_checkpoint)?
    };
    let dump_writer = output_dir
        .map(|dir| DumpDirWriter::new(dir, &prev_checkpoint))
        .transpose()?;
    let mut dump_processor = if no_sync {
        DumpProcessor::new_no_sync(dump_writer)
    } else {
        DumpProcessor::new_sync(dump_writer, store, &prev_checkpoint)
    };
    let mut should_continue = true;
    let mut dumps_since_checkpoint = 0;
    // Fetch and process dumps until we have the full block state
    while should_continue {
        let dump = dump_reader.read_dump().await?;
        should_continue = dump_processor.process_dump(dump).await?;
        // Write checkpoint every `DUMPS_BEFORE_CHECKPOINT` dumps if we have one
        if let Some(checkpoint_filename) = checkpoint.as_ref() {
            dumps_since_checkpoint += 1;
            if dumps_since_checkpoint >= DUMPS_BEFORE_CHECKPOINT || !should_continue {
                dumps_since_checkpoint = 0;
                let checkpoint = CheckPoint {
                    processing: dump_processor.get_checkpoint(),
                    reading: dump_reader.get_checkpoint(),
                };
                let checkpoint_file = File::create(checkpoint_filename)?;
                serde_json::to_writer(checkpoint_file, &checkpoint)?;
            }
        }
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
            .unwrap_or_else(|| keccak(address));
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
    fn new_sync(
        writer: Option<DumpDirWriter>,
        store: Store,
        prev_checkpoint: &Option<CheckPoint>,
    ) -> Self {
        Self {
            state_root: None,
            sync_state: Some((
                prev_checkpoint
                    .as_ref()
                    .and_then(|check_point| check_point.processing.current_root)
                    .unwrap_or(*EMPTY_TRIE_HASH),
                store,
            )),
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

    fn get_checkpoint(&self) -> ProcessingCheckpoint {
        ProcessingCheckpoint {
            current_root: self
                .sync_state
                .as_ref()
                .map(|(current_root, _)| *current_root),
            current_file: self.writer.as_ref().map(|writer| writer.current_file),
        }
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
    fn new(dirname: String, prev_checkpoint: &Option<CheckPoint>) -> eyre::Result<DumpDirWriter> {
        if !std::path::Path::new(&dirname).exists() {
            std::fs::create_dir(&dirname)?;
        }
        Ok(Self {
            dirname,
            current_file: prev_checkpoint
                .as_ref()
                .and_then(|checkpoint| checkpoint.processing.current_file)
                .unwrap_or_default(),
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
    fn new_from_dir(dirname: String, prev_checkpoint: &Option<CheckPoint>) -> eyre::Result<Self> {
        let mut dir_reader = DumpDirReader::new(dirname)?;
        if let Some(checkpoint) = prev_checkpoint {
            if let Some(current) = checkpoint.reading.current_file {
                dir_reader.current_file = current
            }
        }
        Ok(Self::Dir(dir_reader))
    }

    /// Create a new DumpReader that will read state data from an archive node given the path to its IPC file
    async fn new_from_ipc(
        archive_ipc_path: &str,
        block_number: BlockNumber,
        prev_checkpoint: &Option<CheckPoint>,
    ) -> eyre::Result<Self> {
        let mut ipc_reader = DumpIpcReader::new(archive_ipc_path, block_number).await?;
        if let Some(checkpoint) = prev_checkpoint {
            if let Some(start) = checkpoint.reading.start_hash {
                ipc_reader.start = start
            }
        }
        Ok(Self::Ipc(ipc_reader))
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

    fn get_checkpoint(&self) -> ReadingCheckpoint {
        let mut checkpoint = ReadingCheckpoint::default();
        match self {
            DumpReader::Dir(dir_reader) => checkpoint.current_file = Some(dir_reader.current_file),
            DumpReader::Ipc(ipc_reader) => checkpoint.start_hash = Some(ipc_reader.start),
        }
        checkpoint
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
        "params": [format!("{:#x}", self.block_number), format!("{:#x}", self.start), MAX_ACCOUNTS, false, false, false]
        });
        let response = send_ipc_json_request(&mut self.stream, request).await?;
        let dump: Dump = serde_json::from_value(response)?;
        // Find the next hash
        let last_key = dump
            .accounts
            .iter()
            .map(|(addr, acc)| acc.hashed_address.unwrap_or_else(|| keccak(addr)))
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

#[derive(Deserialize, Debug, Serialize, Default)]
struct CheckPoint {
    processing: ProcessingCheckpoint,
    reading: ReadingCheckpoint,
}

#[derive(Deserialize, Debug, Serialize, Default)]
struct ProcessingCheckpoint {
    current_root: Option<H256>,
    current_file: Option<usize>,
}

#[derive(Deserialize, Debug, Serialize, Default)]
struct ReadingCheckpoint {
    start_hash: Option<H256>,
    current_file: Option<usize>,
}

/// Loads previous checkpoint (if it exists)
fn load_checkpoint(
    checkpoint: &Option<String>,
    ipc_input: bool,
    file_input: bool,
    file_output: bool,
    no_sync: bool,
) -> Result<Option<CheckPoint>, eyre::Error> {
    let prev_checkpoint: Option<CheckPoint> = match checkpoint {
        Some(checkpoint_filename) if std::path::Path::new(&checkpoint_filename).exists() => {
            Some(serde_json::from_reader(File::open(checkpoint_filename)?)?)
        }
        _ => None,
    };
    // Validate current flags match pre-existing state
    if let Some(checkpoint) = &prev_checkpoint {
        info!("Checkpoint detected, resuming archive sync");
        debug!("Previous checkpoint: {checkpoint:?}");
        // Check inconsistencies
        if ipc_input && checkpoint.reading.start_hash.is_none()
            || file_input && checkpoint.reading.current_file.is_none()
        {
            return Err(eyre::Error::msg("Input type doesn't match checkpoint data"));
        }
        if file_output && checkpoint.processing.current_file.is_none() {
            return Err(eyre::Error::msg(
                "Output directory received but no current file found in checkpoint",
            ));
        }
        if !no_sync && checkpoint.processing.current_root.is_none() {
            return Err(eyre::Error::msg(
                "Checkpoint file doesn't contain currnet root, try running with --no_sync",
            ));
        }
        // Warn and request user approval before resuming a sync process with --no_sync flag
        if no_sync && checkpoint.processing.current_root.is_some() {
            println!(
                "Previous archive sync started a state sync, are you sure you want to continue with --no_sync? Please type `confirm`"
            );
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("confirm") {
                return Err(eyre::Error::msg("Archive sync cancelled"));
            }
        }
        // Warn and request user approval before resuming a sync process that wrote state to files without --output_dir
        if !file_output && checkpoint.processing.current_file.is_some() {
            println!(
                "Previous archive sync downloaded state to files, are you sure you want to continue without an output_dir? Please type `confirm`"
            );
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("confirm") {
                return Err(eyre::Error::msg("Archive sync cancelled"));
            }
        }
    }
    Ok(prev_checkpoint)
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
    #[arg(
        long = "checkpoint",
        value_name = "CHECKPOINT_FILENAME",
        help = "Receives the name of the file where the checkpoint is/will be located",
        long_help = "Receives the name of the file where the checkpoint is/will be located. This checkpoint will be used to resume a previous archive sync process if aborted"
    )]
    pub checkpoint: Option<String>,
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
        args.checkpoint,
        store,
    )
    .await
}
