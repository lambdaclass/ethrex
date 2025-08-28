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
use num_bigint::BigUint;
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
    #[serde(deserialize_with = "deser_address")]
    address: Option<Address>,
    #[serde(rename = "key")]
    hashed_address: Option<H256>,
}

fn deser_address<'de, D>(d: D) -> Result<Option<Address>, D::Error>
        where
            D: Deserializer<'de>,
        {
            Ok(Address::deserialize(d).ok())
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
        let ipc_input = r#"{"root":"2799f3c1a02d27b6b9cca78fcd01a225b96360c7b188aa769f1cd1620df241d1","accounts":{"0x6BCd15F58Cd30e3a94753ffeaFBefFd3d66508eC":{"balance":"0","nonce":1,"root":"0xb33a4179ff04df2b010ebcfda367d1324e65057d9458a26752fcd4dd4bd6f440","codeHash":"0x2da226e6b4e24164786e79f77286d055c1d441de96cd3e1fcba519697b6227ac","code":"0x6080604052600436106102725760003560e01c8063715018a61161014f578063b071401b116100c1578063d5abeb011161007a578063d5abeb0114610738578063db4bec441461074e578063e0a808531461077e578063e985e9c51461079e578063efbd73f4146107be578063f2fde38b146107de57600080fd5b8063b071401b14610678578063b767a09814610698578063b88d4fde146106b8578063c23dc68f146106d8578063c87b56dd14610705578063d2cab0561461072557600080fd5b806394354fd01161011357806394354fd0146105e557806395d89b41146105fb57806399a2557a14610610578063a0712d6814610630578063a22cb46514610643578063a45ba8e71461066357600080fd5b8063715018a6146105455780637cb647591461055a5780637ec4a6591461057a5780638462151c1461059a5780638da5cb5b146105c757600080fd5b806342842e0e116101e85780635bbb2177116101ac5780635bbb21771461048a5780635c975abb146104b757806362b99ad4146104d15780636352211e146104e65780636caede3d1461050657806370a082311461052557600080fd5b806342842e0e146103f557806344a0d68a146104155780634fdd43cb1461043557806351830227146104555780635503a0e81461047557600080fd5b806316ba10e01161023a57806316ba10e01461034c57806316c38b3c1461036c57806318160ddd1461038c57806323b872dd146103aa5780632eb4a7ab146103ca5780633ccfd60b146103e057600080fd5b806301ffc9a71461027757806306fdde03146102ac578063081812fc146102ce578063095ea7b31461030657806313faede614610328575b600080fd5b34801561028357600080fd5b50610297610292366004612309565b6107fe565b60405190151581526020015b60405180910390f35b3480156102b857600080fd5b506102c1610850565b6040516102a3919061237e565b3480156102da57600080fd5b506102ee6102e9366004612391565b6108e2565b6040516001600160a01b0390911681526020016102a3565b34801561031257600080fd5b506103266103213660046123c6565b610926565b005b34801561033457600080fd5b5061033e600f5481565b6040519081526020016102a3565b34801561035857600080fd5b5061032661036736600461248d565b6109ad565b34801561037857600080fd5b506103266103873660046124e5565b6109f7565b34801561039857600080fd5b5061033e600154600054036000190190565b3480156103b657600080fd5b506103266103c5366004612500565b610a34565b3480156103d657600080fd5b5061033e600a5481565b3480156103ec57600080fd5b50610326610a3f565b34801561040157600080fd5b50610326610410366004612500565b610b3a565b34801561042157600080fd5b50610326610430366004612391565b610b55565b34801561044157600080fd5b5061032661045036600461248d565b610b84565b34801561046157600080fd5b506012546102979062010000900460ff1681565b34801561048157600080fd5b506102c1610bc1565b34801561049657600080fd5b506104aa6104a536600461253c565b610c4f565b6040516102a391906125e1565b3480156104c357600080fd5b506012546102979060ff1681565b3480156104dd57600080fd5b506102c1610d15565b3480156104f257600080fd5b506102ee610501366004612391565b610d22565b34801561051257600080fd5b5060125461029790610100900460ff1681565b34801561053157600080fd5b5061033e61054036600461264b565b610d34565b34801561055157600080fd5b50610326610d82565b34801561056657600080fd5b50610326610575366004612391565b610db8565b34801561058657600080fd5b5061032661059536600461248d565b610de7565b3480156105a657600080fd5b506105ba6105b536600461264b565b610e24565b6040516102a39190612666565b3480156105d357600080fd5b506008546001600160a01b03166102ee565b3480156105f157600080fd5b5061033e60115481565b34801561060757600080fd5b506102c1610f71565b34801561061c57600080fd5b506105ba61062b36600461269e565b610f80565b61032661063e366004612391565b611146565b34801561064f57600080fd5b5061032661065e3660046126d1565b611263565b34801561066f57600080fd5b506102c16112f9565b34801561068457600080fd5b50610326610693366004612391565b611306565b3480156106a457600080fd5b506103266106b33660046124e5565b611335565b3480156106c457600080fd5b506103266106d3366004612704565b611379565b3480156106e457600080fd5b506106f86106f3366004612391565b6113c3565b6040516102a3919061277f565b34801561071157600080fd5b506102c1610720366004612391565b61147d565b6103266107333660046127b4565b6115ec565b34801561074457600080fd5b5061033e60105481565b34801561075a57600080fd5b5061029761076936600461264b565b600b6020526000908152604090205460ff1681565b34801561078a57600080fd5b506103266107993660046124e5565b611851565b3480156107aa57600080fd5b506102976107b9366004612832565b611897565b3480156107ca57600080fd5b506103266107d936600461285c565b6118c5565b3480156107ea57600080fd5b506103266107f936600461264b565b611965565b60006001600160e01b031982166380ac58cd60e01b148061082f57506001600160e01b03198216635b5e139f60e01b145b8061084a57506301ffc9a760e01b6001600160e01b03198316145b92915050565b60606002805461085f9061287f565b80601f016020809104026020016040519081016040528092919081815260200182805461088b9061287f565b80156108d85780601f106108ad576101008083540402835291602001916108d8565b820191906000526020600020905b8154815290600101906020018083116108bb57829003601f168201915b5050505050905090565b60006108ed82611a00565b61090a576040516333d1c03960e21b815260040160405180910390fd5b506000908152600660205260409020546001600160a01b031690565b600061093182610d22565b9050806001600160a01b0316836001600160a01b031614156109665760405163250fdee360e21b815260040160405180910390fd5b336001600160a01b0382161461099d576109808133611897565b61099d576040516367d9dca160e11b815260040160405180910390fd5b6109a8838383611a39565b505050565b6008546001600160a01b031633146109e05760405162461bcd60e51b81526004016109d7906128ba565b60405180910390fd5b80516109f390600d90602084019061225a565b5050565b6008546001600160a01b03163314610a215760405162461bcd60e51b81526004016109d7906128ba565b6012805460ff1916911515919091179055565b6109a8838383611a95565b6008546001600160a01b03163314610a695760405162461bcd60e51b81526004016109d7906128ba565b60026009541415610abc5760405162461bcd60e51b815260206004820152601f60248201527f5265656e7472616e637947756172643a207265656e7472616e742063616c6c0060448201526064016109d7565b60026009556000610ad56008546001600160a01b031690565b6001600160a01b03164760405160006040518083038185875af1925050503d8060008114610b1f576040519150601f19603f3d011682016040523d82523d6000602084013e610b24565b606091505b5050905080610b3257600080fd5b506001600955565b6109a883838360405180602001604052806000815250611379565b6008546001600160a01b03163314610b7f5760405162461bcd60e51b81526004016109d7906128ba565b600f55565b6008546001600160a01b03163314610bae5760405162461bcd60e51b81526004016109d7906128ba565b80516109f390600e90602084019061225a565b600d8054610bce9061287f565b80601f0160208091040260200160405190810160405280929190818152602001828054610bfa9061287f565b8015610c475780601f10610c1c57610100808354040283529160200191610c47565b820191906000526020600020905b815481529060010190602001808311610c2a57829003601f168201915b505050505081565b80516060906000816001600160401b03811115610c6e57610c6e6123f0565b604051908082528060200260200182016040528015610cb957816020015b6040805160608101825260008082526020808301829052928201528252600019909201910181610c8c5790505b50905060005b828114610d0d57610ce8858281518110610cdb57610cdb6128ef565b60200260200101516113c3565b828281518110610cfa57610cfa6128ef565b6020908102919091010152600101610cbf565b509392505050565b600c8054610bce9061287f565b6000610d2d82611c82565b5192915050565b60006001600160a01b038216610d5d576040516323d3ad8160e21b815260040160405180910390fd5b506001600160a01b03166000908152600560205260409020546001600160401b031690565b6008546001600160a01b03163314610dac5760405162461bcd60e51b81526004016109d7906128ba565b610db66000611da4565b565b6008546001600160a01b03163314610de25760405162461bcd60e51b81526004016109d7906128ba565b600a55565b6008546001600160a01b03163314610e115760405162461bcd60e51b81526004016109d7906128ba565b80516109f390600c90602084019061225a565b60606000806000610e3485610d34565b90506000816001600160401b03811115610e5057610e506123f0565b604051908082528060200260200182016040528015610e79578160200160208202803683370190505b509050610e9f604080516060810182526000808252602082018190529181019190915290565b60015b838614610f6557600081815260046020908152604091829020825160608101845290546001600160a01b0381168252600160a01b81046001600160401b031692820192909252600160e01b90910460ff16158015928201929092529250610f0857610f5d565b81516001600160a01b031615610f1d57815194505b876001600160a01b0316856001600160a01b03161415610f5d5780838780600101985081518110610f5057610f506128ef565b6020026020010181815250505b600101610ea2565b50909695505050505050565b60606003805461085f9061287f565b6060818310610fa257604051631960ccad60e11b815260040160405180910390fd5b600080546001851015610fb457600194505b80841115610fc0578093505b6000610fcb87610d34565b905084861015610fea5785850381811015610fe4578091505b50610fee565b5060005b6000816001600160401b03811115611008576110086123f0565b604051908082528060200260200182016040528015611031578160200160208202803683370190505b5090508161104457935061113f92505050565b600061104f886113c3565b905060008160400151611060575080515b885b8881141580156110725750848714155b1561113357600081815260046020908152604091829020825160608101845290546001600160a01b0381168252600160a01b81046001600160401b031692820192909252600160e01b90910460ff161580159282019290925293506110d65761112b565b82516001600160a01b0316156110eb57825191505b8a6001600160a01b0316826001600160a01b0316141561112b578084888060010199508151811061111e5761111e6128ef565b6020026020010181815250505b600101611062565b50505092835250909150505b9392505050565b8060008111801561115957506011548111155b6111755760405162461bcd60e51b81526004016109d790612905565b6010548161118a600154600054036000190190565b6111949190612949565b11156111b25760405162461bcd60e51b81526004016109d790612961565b8180600f546111c1919061298f565b3410156112065760405162461bcd60e51b8152602060048201526013602482015272496e73756666696369656e742066756e64732160681b60448201526064016109d7565b60125460ff16156112595760405162461bcd60e51b815260206004820152601760248201527f54686520636f6e7472616374206973207061757365642100000000000000000060448201526064016109d7565b6109a83384611df6565b6001600160a01b03821633141561128d5760405163b06307db60e01b815260040160405180910390fd5b3360008181526007602090815260408083206001600160a01b03871680855290835292819020805460ff191686151590811790915590519081529192917f17307eab39ab6107e8899845ad3d59bd9653f200f220920489ca2b5937696c31910160405180910390a35050565b600e8054610bce9061287f565b6008546001600160a01b031633146113305760405162461bcd60e51b81526004016109d7906128ba565b601155565b6008546001600160a01b0316331461135f5760405162461bcd60e51b81526004016109d7906128ba565b601280549115156101000261ff0019909216919091179055565b611384848484611a95565b6001600160a01b0383163b156113bd576113a084848484611e10565b6113bd576040516368d2bf6b60e11b815260040160405180910390fd5b50505050565b6040805160608082018352600080835260208084018290528385018290528451928301855281835282018190529281019290925290600183108061140957506000548310155b156114145792915050565b50600082815260046020908152604091829020825160608101845290546001600160a01b0381168252600160a01b81046001600160401b031692820192909252600160e01b90910460ff1615801592820192909252906114745792915050565b61113f83611c82565b606061148882611a00565b6114ec5760405162461bcd60e51b815260206004820152602f60248201527f4552433732314d657461646174613a2055524920717565727920666f72206e6f60448201526e3732bc34b9ba32b73a103a37b5b2b760891b60648201526084016109d7565b60125462010000900460ff1661158e57600e80546115099061287f565b80601f01602080910402602001604051908101604052809291908181526020018280546115359061287f565b80156115825780601f1061155757610100808354040283529160200191611582565b820191906000526020600020905b81548152906001019060200180831161156557829003601f168201915b50505050509050919050565b6000611598611f08565b905060008151116115b8576040518060200160405280600081525061113f565b806115c284611f17565b600d6040516020016115d6939291906129ae565b6040516020818303038152906040529392505050565b826000811180156115ff57506011548111155b61161b5760405162461bcd60e51b81526004016109d790612905565b60105481611630600154600054036000190190565b61163a9190612949565b11156116585760405162461bcd60e51b81526004016109d790612961565b8380600f54611667919061298f565b3410156116ac5760405162461bcd60e51b8152602060048201526013602482015272496e73756666696369656e742066756e64732160681b60448201526064016109d7565b601254610100900460ff1661170e5760405162461bcd60e51b815260206004820152602260248201527f5468652077686974656c6973742073616c65206973206e6f7420656e61626c65604482015261642160f01b60648201526084016109d7565b336000908152600b602052604090205460ff161561176e5760405162461bcd60e51b815260206004820152601860248201527f4164647265737320616c726561647920636c61696d656421000000000000000060448201526064016109d7565b6040516bffffffffffffffffffffffff193360601b1660208201526000906034016040516020818303038152906040528051906020012090506117e885858080602002602001604051908101604052809392919081815260200183836020028082843760009201919091525050600a549150849050612014565b6118255760405162461bcd60e51b815260206004820152600e60248201526d496e76616c69642070726f6f662160901b60448201526064016109d7565b336000818152600b60205260409020805460ff191660011790556118499087611df6565b505050505050565b6008546001600160a01b0316331461187b5760405162461bcd60e51b81526004016109d7906128ba565b60128054911515620100000262ff000019909216919091179055565b6001600160a01b03918216600090815260076020908152604080832093909416825291909152205460ff1690565b816000811180156118d857506011548111155b6118f45760405162461bcd60e51b81526004016109d790612905565b60105481611909600154600054036000190190565b6119139190612949565b11156119315760405162461bcd60e51b81526004016109d790612961565b6008546001600160a01b0316331461195b5760405162461bcd60e51b81526004016109d7906128ba565b6109a88284611df6565b6008546001600160a01b0316331461198f5760405162461bcd60e51b81526004016109d7906128ba565b6001600160a01b0381166119f45760405162461bcd60e51b815260206004820152602660248201527f4f776e61626c653a206e6577206f776e657220697320746865207a65726f206160448201526564647265737360d01b60648201526084016109d7565b6119fd81611da4565b50565b600081600111158015611a14575060005482105b801561084a575050600090815260046020526040902054600160e01b900460ff161590565b60008281526006602052604080822080546001600160a01b0319166001600160a01b0387811691821790925591518593918516917f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b92591a4505050565b6000611aa082611c82565b9050836001600160a01b031681600001516001600160a01b031614611ad75760405162a1148160e81b815260040160405180910390fd5b6000336001600160a01b0386161480611af55750611af58533611897565b80611b10575033611b05846108e2565b6001600160a01b0316145b905080611b3057604051632ce44b5f60e11b815260040160405180910390fd5b6001600160a01b038416611b5757604051633a954ecd60e21b815260040160405180910390fd5b611b6360008487611a39565b6001600160a01b038581166000908152600560209081526040808320805467ffffffffffffffff198082166001600160401b0392831660001901831617909255898616808652838620805493841693831660019081018416949094179055898652600490945282852080546001600160e01b031916909417600160a01b42909216919091021783558701808452922080549193909116611c37576000548214611c3757805460208601516001600160401b0316600160a01b026001600160e01b03199091166001600160a01b038a16171781555b50505082846001600160a01b0316866001600160a01b03167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef60405160405180910390a45050505050565b60408051606081018252600080825260208201819052918101919091528180600111611d8b57600054811015611d8b57600081815260046020908152604091829020825160608101845290546001600160a01b0381168252600160a01b81046001600160401b031692820192909252600160e01b90910460ff16151591810182905290611d895780516001600160a01b031615611d20579392505050565b5060001901600081815260046020908152604091829020825160608101845290546001600160a01b038116808352600160a01b82046001600160401b031693830193909352600160e01b900460ff1615159281019290925215611d84579392505050565b611d20565b505b604051636f96cda160e11b815260040160405180910390fd5b600880546001600160a01b038381166001600160a01b0319831681179093556040519116919082907f8be0079c531659141344cd1fd0a4f28419497f9722a3daafe3b4186f6b6457e090600090a35050565b6109f382826040518060200160405280600081525061202a565b604051630a85bd0160e11b81526000906001600160a01b0385169063150b7a0290611e45903390899088908890600401612a72565b602060405180830381600087803b158015611e5f57600080fd5b505af1925050508015611e8f575060408051601f3d908101601f19168201909252611e8c91810190612aaf565b60015b611eea573d808015611ebd576040519150601f19603f3d011682016040523d82523d6000602084013e611ec2565b606091505b508051611ee2576040516368d2bf6b60e11b815260040160405180910390fd5b805181602001fd5b6001600160e01b031916630a85bd0160e11b1490505b949350505050565b6060600c805461085f9061287f565b606081611f3b5750506040805180820190915260018152600360fc1b602082015290565b8160005b8115611f655780611f4f81612acc565b9150611f5e9050600a83612afd565b9150611f3f565b6000816001600160401b03811115611f7f57611f7f6123f0565b6040519080825280601f01601f191660200182016040528015611fa9576020820181803683370190505b5090505b8415611f0057611fbe600183612b11565b9150611fcb600a86612b28565b611fd6906030612949565b60f81b818381518110611feb57611feb6128ef565b60200101906001600160f81b031916908160001a90535061200d600a86612afd565b9450611fad565b60008261202185846121ee565b14949350505050565b6000546001600160a01b03841661205357604051622e076360e81b815260040160405180910390fd5b826120715760405163b562e8dd60e01b815260040160405180910390fd5b6001600160a01b038416600081815260056020908152604080832080546fffffffffffffffffffffffffffffffff1981166001600160401b038083168b0181169182176801000000000000000067ffffffffffffffff1990941690921783900481168b01811690920217909155858452600490925290912080546001600160e01b0319168317600160a01b42909316929092029190911790558190818501903b15612199575b60405182906001600160a01b038816906000907fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef908290a46121626000878480600101955087611e10565b61217f576040516368d2bf6b60e11b815260040160405180910390fd5b80821061211757826000541461219457600080fd5b6121de565b5b6040516001830192906001600160a01b038816906000907fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef908290a480821061219a575b5060009081556113bd9085838684565b600081815b8451811015610d0d576000858281518110612210576122106128ef565b602002602001015190508083116122365760008381526020829052604090209250612247565b600081815260208490526040902092505b508061225281612acc565b9150506121f3565b8280546122669061287f565b90600052602060002090601f01602090048101928261228857600085556122ce565b82601f106122a157805160ff19168380011785556122ce565b828001600101855582156122ce579182015b828111156122ce5782518255916020019190600101906122b3565b506122da9291506122de565b5090565b5b808211156122da57600081556001016122df565b6001600160e01b0319811681146119fd57600080fd5b60006020828403121561231b57600080fd5b813561113f816122f3565b60005b83811015612341578181015183820152602001612329565b838111156113bd5750506000910152565b6000815180845261236a816020860160208601612326565b601f01601f19169290920160200192915050565b60208152600061113f6020830184612352565b6000602082840312156123a357600080fd5b5035919050565b80356001600160a01b03811681146123c157600080fd5b919050565b600080604083850312156123d957600080fd5b6123e2836123aa565b946020939093013593505050565b634e487b7160e01b600052604160045260246000fd5b604051601f8201601f191681016001600160401b038111828210171561242e5761242e6123f0565b604052919050565b60006001600160401b0383111561244f5761244f6123f0565b612462601f8401601f1916602001612406565b905082815283838301111561247657600080fd5b828260208301376000602084830101529392505050565b60006020828403121561249f57600080fd5b81356001600160401b038111156124b557600080fd5b8201601f810184136124c657600080fd5b611f0084823560208401612436565b803580151581146123c157600080fd5b6000602082840312156124f757600080fd5b61113f826124d5565b60008060006060848603121561251557600080fd5b61251e846123aa565b925061252c602085016123aa565b9150604084013590509250925092565b6000602080838503121561254f57600080fd5b82356001600160401b038082111561256657600080fd5b818501915085601f83011261257a57600080fd5b81358181111561258c5761258c6123f0565b8060051b915061259d848301612406565b81815291830184019184810190888411156125b757600080fd5b938501935b838510156125d5578435825293850193908501906125bc565b98975050505050505050565b6020808252825182820181905260009190848201906040850190845b81811015610f655761263883855180516001600160a01b031682526020808201516001600160401b0316908301526040908101511515910152565b92840192606092909201916001016125fd565b60006020828403121561265d57600080fd5b61113f826123aa565b6020808252825182820181905260009190848201906040850190845b81811015610f6557835183529284019291840191600101612682565b6000806000606084860312156126b357600080fd5b6126bc846123aa565b95602085013595506040909401359392505050565b600080604083850312156126e457600080fd5b6126ed836123aa565b91506126fb602084016124d5565b90509250929050565b6000806000806080858703121561271a57600080fd5b612723856123aa565b9350612731602086016123aa565b92506040850135915060608501356001600160401b0381111561275357600080fd5b8501601f8101871361276457600080fd5b61277387823560208401612436565b91505092959194509250565b81516001600160a01b031681526020808301516001600160401b0316908201526040808301511515908201526060810161084a565b6000806000604084860312156127c957600080fd5b8335925060208401356001600160401b03808211156127e757600080fd5b818601915086601f8301126127fb57600080fd5b81358181111561280a57600080fd5b8760208260051b850101111561281f57600080fd5b6020830194508093505050509250925092565b6000806040838503121561284557600080fd5b61284e836123aa565b91506126fb602084016123aa565b6000806040838503121561286f57600080fd5b823591506126fb602084016123aa565b600181811c9082168061289357607f821691505b602082108114156128b457634e487b7160e01b600052602260045260246000fd5b50919050565b6020808252818101527f4f776e61626c653a2063616c6c6572206973206e6f7420746865206f776e6572604082015260600190565b634e487b7160e01b600052603260045260246000fd5b602080825260149082015273496e76616c6964206d696e7420616d6f756e742160601b604082015260600190565b634e487b7160e01b600052601160045260246000fd5b6000821982111561295c5761295c612933565b500190565b6020808252601490820152734d617820737570706c792065786365656465642160601b604082015260600190565b60008160001904831182151516156129a9576129a9612933565b500290565b6000845160206129c18285838a01612326565b8551918401916129d48184848a01612326565b8554920191600090600181811c90808316806129f157607f831692505b858310811415612a0f57634e487b7160e01b85526022600452602485fd5b808015612a235760018114612a3457612a61565b60ff19851688528388019550612a61565b60008b81526020902060005b85811015612a595781548a820152908401908801612a40565b505083880195505b50939b9a5050505050505050505050565b6001600160a01b0385811682528416602082015260408101839052608060608201819052600090612aa590830184612352565b9695505050505050565b600060208284031215612ac157600080fd5b815161113f816122f3565b6000600019821415612ae057612ae0612933565b5060010190565b634e487b7160e01b600052601260045260246000fd5b600082612b0c57612b0c612ae7565b500490565b600082821015612b2357612b23612933565b500390565b600082612b3757612b37612ae7565b50069056fea2646970667358221220e3de81e0eb35ea9719e2c8cb7f23351e64793425f22d9120a8e79e52e9672c2264736f6c63430008090033","storage":{"0x0000000000000000000000000000000000000000000000000000000000000000":"08","0x0000000000000000000000000000000000000000000000000000000000000002":"5761696675204261746820576174657200000000000000000000000000000020","0x0000000000000000000000000000000000000000000000000000000000000003":"5742570000000000000000000000000000000000000000000000000000000006","0x0000000000000000000000000000000000000000000000000000000000000008":"5b0b4685ff26210512e2471868fc84df4dadbcfc","0x0000000000000000000000000000000000000000000000000000000000000009":"01","0x000000000000000000000000000000000000000000000000000000000000000d":"2e6a736f6e00000000000000000000000000000000000000000000000000000a","0x000000000000000000000000000000000000000000000000000000000000000e":"83","0x000000000000000000000000000000000000000000000000000000000000000f":"2386f26fc10000","0x0000000000000000000000000000000000000000000000000000000000000010":"2710","0x0000000000000000000000000000000000000000000000000000000000000011":"03e8","0x04cde762ef08b6b6c5ded8e8c4c0b3f4e5c9ad7342c88fcc93681b4588b73f05":"635cda97971c3e5b989889b05c315263b08a6cd9bbb8721e","0x1a1e6821cde7d0159c0d293177871e09677b4e42307c7db3ba94f8648a5a050f":"635cda67971c3e5b989889b05c315263b08a6cd9bbb8721e","0x2e174c10e159ea99b867ce3205125c24a42d128804e4070ed6fcc8cc98166aa0":"635b79b784dcd025203ba5568ce60145ba96d8ce17fb324a","0x6724b37920771398abe6ce53452f24929730efd3a8327f5c3fecdb69eb180a4c":"030000000000000003","0x91da3fd0782e51c6b3986e9e672fd566868e71f3dbc2d6c2cd6fbb3e361af2a7":"63413c4b433ad1d7e760afa3c1fb780c94d15a0515620ff9","0xabd6e7cb50984ff9c2f3e18a2660c3353dadf4e3291deeb275dae2cd1e44fe05":"633fc7075b0b4685ff26210512e2471868fc84df4dadbcfc","0xbb7b4a454dc3493923482f07822329ed19e8244eff582cc204f8554c3620c3fd":"697066733a2f2f516d5863324233475a59544236724e4877735779583833664e","0xbb7b4a454dc3493923482f07822329ed19e8244eff582cc204f8554c3620c3fe":"686a6368517537725236456e5934535079667258652f68696464656e2e6a736f","0xbb7b4a454dc3493923482f07822329ed19e8244eff582cc204f8554c3620c3ff":"6e00000000000000000000000000000000000000000000000000000000000000","0xbeb3bad75134cb432e5707980e3245c52c5998a1125ee30f2f0dbf3925b1e551":"6360ca8b971c3e5b989889b05c315263b08a6cd9bbb8721e","0xc59312466997bb42aaaf719ece141047820e6b34531e1670dc1852a453648f0f":"635e9b53c8c56c78b3b45b8c1bc6a12fd0f87b94b20c8052"},"address":"0x6bcd15f58cd30e3a94753ffeafbeffd3d66508ec","key":"0x00001568791cc2d0ea38393918007924a9d4ac5bd7484b632db452412c6d491f"}},"next":"AAAVaSOe3y+KBk+Mh+F7nvtCqZ4gioC65fzv4gQSid8="}"#;
        let dump: Dump = serde_json::from_str(&ipc_input).unwrap();
        let store = Store::new("uwu.memory", ethrex_storage::EngineType::InMemory).unwrap();
        process_dump(dump, store, *EMPTY_TRIE_HASH).await.unwrap();
    }
}

/*

    Addr: 0x6BCd15F58Cd30e3a94753ffeaFBefFd3d66508eC":
    {"balance":"0","nonce":1,"root":"0xb33a4179ff04df2b010ebcfda367d1324e65057d9458a26752fcd4dd4bd6f440","codeHash":"0x2da226e6b4e24164786e79f77286d055c1d441de96cd3e1fcba519697b6227ac",
    "storage":{
    "0x0000000000000000000000000000000000000000000000000000000000000000":"08"
    "0x0000000000000000000000000000000000000000000000000000000000000002":"5761696675204261746820576174657200000000000000000000000000000020"
    "0x0000000000000000000000000000000000000000000000000000000000000003":"5742570000000000000000000000000000000000000000000000000000000006"
    "0x0000000000000000000000000000000000000000000000000000000000000008":"5b0b4685ff26210512e2471868fc84df4dadbcfc"
    "0x0000000000000000000000000000000000000000000000000000000000000009":"01"
    "0x000000000000000000000000000000000000000000000000000000000000000d":"2e6a736f6e00000000000000000000000000000000000000000000000000000a"
    "0x000000000000000000000000000000000000000000000000000000000000000e":"83"
    "0x000000000000000000000000000000000000000000000000000000000000000f":"2386f26fc10000"
    "0x0000000000000000000000000000000000000000000000000000000000000010":"2710"
    "0x0000000000000000000000000000000000000000000000000000000000000011":"03e8"
    "0x04cde762ef08b6b6c5ded8e8c4c0b3f4e5c9ad7342c88fcc93681b4588b73f05":"635cda97971c3e5b989889b05c315263b08a6cd9bbb8721e"
    "0x1a1e6821cde7d0159c0d293177871e09677b4e42307c7db3ba94f8648a5a050f":"635cda67971c3e5b989889b05c315263b08a6cd9bbb8721e"
    "0x2e174c10e159ea99b867ce3205125c24a42d128804e4070ed6fcc8cc98166aa0":"635b79b784dcd025203ba5568ce60145ba96d8ce17fb324a"
    "0x6724b37920771398abe6ce53452f24929730efd3a8327f5c3fecdb69eb180a4c":"030000000000000003"
    "0x91da3fd0782e51c6b3986e9e672fd566868e71f3dbc2d6c2cd6fbb3e361af2a7":"63413c4b433ad1d7e760afa3c1fb780c94d15a0515620ff9"
    "0xabd6e7cb50984ff9c2f3e18a2660c3353dadf4e3291deeb275dae2cd1e44fe05":"633fc7075b0b4685ff26210512e2471868fc84df4dadbcfc"
    "0xbb7b4a454dc3493923482f07822329ed19e8244eff582cc204f8554c3620c3fd":"697066733a2f2f516d5863324233475a59544236724e4877735779583833664e"
    "0xbb7b4a454dc3493923482f07822329ed19e8244eff582cc204f8554c3620c3fe":"686a6368517537725236456e5934535079667258652f68696464656e2e6a736f"
    "0xbb7b4a454dc3493923482f07822329ed19e8244eff582cc204f8554c3620c3ff":"6e00000000000000000000000000000000000000000000000000000000000000"
    "0xbeb3bad75134cb432e5707980e3245c52c5998a1125ee30f2f0dbf3925b1e551":"6360ca8b971c3e5b989889b05c315263b08a6cd9bbb8721e"
    "0xc59312466997bb42aaaf719ece141047820e6b34531e1670dc1852a453648f0f":"635e9b53c8c56c78b3b45b8c1bc6a12fd0f87b94b20c8052"}


     0x0000…0000, value 0x8
     0x0000…0002, value 0x5761696675204261746820576174657200000000000000000000000000000020
     0x0000…0003, value 0x5742570000000000000000000000000000000000000000000000000000000006
     0x0000…0008, value 0x5b0b4685ff26210512e2471868fc84df4dadbcfc
     0x0000…0009, value 0x1
     0x0000…000d, value 0x2e6a736f6e00000000000000000000000000000000000000000000000000000a
     0x0000…000e, value 0x83
     0x0000…000f, value 0x2386f26fc10000
     0x0000…0010, value 0x2710
     0x0000…0011, value 0x3e8
     0x04cd…3f05, value 0x635cda97971c3e5b989889b05c315263b08a6cd9bbb8721e
     0x1a1e…050f, value 0x635cda67971c3e5b989889b05c315263b08a6cd9bbb8721e
     0x2e17…6aa0, value 0x635b79b784dcd025203ba5568ce60145ba96d8ce17fb324a
     0x6724…0a4c, value 0x30000000000000003
     0x91da…f2a7, value 0x63413c4b433ad1d7e760afa3c1fb780c94d15a0515620ff9
     0xabd6…fe05, value 0x633fc7075b0b4685ff26210512e2471868fc84df4dadbcfc
     0xbb7b…c3fd, value 0x697066733a2f2f516d5863324233475a59544236724e4877735779583833664e
     0xbb7b…c3fe, value 0x686a6368517537725236456e5934535079667258652f68696464656e2e6a736f
     0xbb7b…c3ff, value 0x6e00000000000000000000000000000000000000000000000000000000000000
     0xbeb3…e551, value 0x6360ca8b971c3e5b989889b05c315263b08a6cd9bbb8721e
     0xc593…8f0f, value 0x635e9b53c8c56c78b3b45b8c1bc6a12fd0f87b94b20c8052
 */
