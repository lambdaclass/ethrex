use std::{
    fmt::Display,
    fs::{File, metadata, read_dir},
    io::{self, BufWriter, Write},
    mem,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant},
};

use clap::{ArgAction, Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::{
    BlockchainOptions, BlockchainType, L2Config,
    error::{ChainError, InvalidBlockError},
};
use ethrex_common::types::{Block, DEFAULT_BUILDER_GAS_CEIL, Genesis, validate_block_body};
use ethrex_p2p::{
    discv4::server::INITIAL_LOOKUP_INTERVAL_MS, peer_table::TARGET_PEERS, sync::SyncMode,
    tx_broadcaster::BROADCAST_INTERVAL_MS, types::Node,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::types::{fork_choice::ForkChoiceState, payload::ExecutionPayload};
use ethrex_storage::{Store, error::StoreError};
use tokio_util::sync::CancellationToken;
use tracing::{Level, error, info, warn};

use crate::{
    initializers::{
        get_network, init_blockchain, init_store, init_tracing, load_store, regenerate_head_state,
    },
    utils::{
        self, default_datadir, get_client_version, get_client_version_string,
        get_minimal_client_version, init_datadir,
    },
};

pub const DB_ETHREX_DEV_L1: &str = "dev_ethrex_l1";

#[cfg(feature = "l2")]
pub const DB_ETHREX_DEV_L2: &str = "dev_ethrex_l2";
use ethrex_config::networks::Network;

#[allow(clippy::upper_case_acronyms)]
#[derive(ClapParser)]
#[command(name="ethrex", author = "Lambdaclass", version=get_client_version_string(), about = "ethrex Execution client")]
pub struct CLI {
    #[command(flatten)]
    pub opts: Options,
    #[command(subcommand)]
    pub command: Option<Subcommand>,
}

#[derive(ClapParser, Debug, Clone)]
pub struct Options {
    #[arg(
        long = "network",
        value_name = "GENESIS_FILE_PATH",
        help = "Receives a `Genesis` struct in json format. You can look at some example genesis files at `fixtures/genesis/*`.",
        long_help = "Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include holesky, sepolia, hoodi and mainnet. If not specified, defaults to mainnet.",
        help_heading = "Node options",
        env = "ETHREX_NETWORK",
        value_parser = clap::value_parser!(Network),
    )]
    pub network: Option<Network>,
    #[arg(long = "bootnodes", value_parser = clap::value_parser!(Node), value_name = "BOOTNODE_LIST", value_delimiter = ',', num_args = 1.., help = "Comma separated enode URLs for P2P discovery bootstrap.", help_heading = "P2P options")]
    pub bootnodes: Vec<Node>,
    #[arg(
        long = "datadir",
        value_name = "DATABASE_DIRECTORY",
        help = "If the datadir is the word `memory`, ethrex will use the InMemory Engine",
        default_value = default_datadir().into_os_string(),
        help = "Receives the name of the directory where the Database is located.",
        long_help = "If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.",
        help_heading = "Node options",
        env = "ETHREX_DATADIR"
    )]
    pub datadir: PathBuf,
    #[arg(
        long = "force",
        help = "Force remove the database",
        long_help = "Delete the database without confirmation.",
        action = clap::ArgAction::SetTrue,
        help_heading = "Node options"
    )]
    pub force: bool,
    #[arg(long = "syncmode", default_value = "snap", value_name = "SYNC_MODE", value_parser = utils::parse_sync_mode, help = "The way in which the node will sync its state.", long_help = "Can be either \"full\" or \"snap\" with \"snap\" as default value.", help_heading = "P2P options")]
    pub syncmode: SyncMode,
    #[arg(
        long = "metrics.addr",
        value_name = "ADDRESS",
        default_value = "0.0.0.0",
        help_heading = "Node options"
    )]
    pub metrics_addr: String,
    #[arg(
        long = "metrics.port",
        value_name = "PROMETHEUS_METRICS_PORT",
        default_value = "9090", // Default Prometheus port (https://prometheus.io/docs/tutorials/getting_started/#show-me-how-it-is-done).
        help_heading = "Node options",
        env = "ETHREX_METRICS_PORT"
    )]
    pub metrics_port: String,
    #[arg(
        long = "metrics",
        action = ArgAction::SetTrue,
        help = "Enable metrics collection and exposition",
        help_heading = "Node options"
    )]
    pub metrics_enabled: bool,
    #[arg(
        long = "dev",
        action = ArgAction::SetTrue,
        help = "Used to create blocks without requiring a Consensus Client",
        long_help = "If set it will be considered as `true`. If `--network` is not specified, it will default to a custom local devnet. The Binary has to be built with the `dev` feature enabled.",
        help_heading = "Node options"
    )]
    pub dev: bool,
    #[arg(
        long = "log.level",
        default_value_t = Level::INFO,
        value_name = "LOG_LEVEL",
        env = "ETHREX_LOG_LEVEL",
        help = "The verbosity level used for logs.",
        long_help = "Possible values: info, debug, trace, warn, error",
        help_heading = "Node options")]
    pub log_level: Level,
    #[arg(
        long = "log.color",
        default_value_t = LogColor::Auto,
        help = "Output logs with ANSI color codes.",
        long_help = "Possible values: auto, always, never",
        help_heading = "Node options"
    )]
    pub log_color: LogColor,
    #[arg(
        long = "log.dir",
        value_name = "LOG_DIR",
        help = "Directory to store log files.",
        help_heading = "Node options"
    )]
    pub log_dir: Option<PathBuf>,
    #[arg(
        help = "Maximum size of the mempool in number of transactions",
        long = "mempool.maxsize",
        default_value_t = 10_000,
        value_name = "MEMPOOL_MAX_SIZE",
        help_heading = "Node options"
    )]
    pub mempool_max_size: usize,
    #[arg(
        long = "http.addr",
        default_value = "0.0.0.0",
        value_name = "ADDRESS",
        help = "Listening address for the http rpc server.",
        help_heading = "RPC options",
        env = "ETHREX_HTTP_ADDR"
    )]
    pub http_addr: String,
    #[arg(
        long = "http.port",
        default_value = "8545",
        value_name = "PORT",
        help = "Listening port for the http rpc server.",
        help_heading = "RPC options",
        env = "ETHREX_HTTP_PORT"
    )]
    pub http_port: String,
    #[arg(
        long = "ws.enabled",
        default_value = "false",
        help = "Enable websocket rpc server. Disabled by default.",
        help_heading = "RPC options",
        env = "ETHREX_ENABLE_WS"
    )]
    pub ws_enabled: bool,
    #[arg(
        long = "ws.addr",
        default_value = "0.0.0.0",
        value_name = "ADDRESS",
        requires = "ws_enabled",
        help = "Listening address for the websocket rpc server.",
        help_heading = "RPC options",
        env = "ETHREX_WS_ADDR"
    )]
    pub ws_addr: String,
    #[arg(
        long = "ws.port",
        default_value = "8546",
        value_name = "PORT",
        requires = "ws_enabled",
        help = "Listening port for the websocket rpc server.",
        help_heading = "RPC options",
        env = "ETHREX_WS_PORT"
    )]
    pub ws_port: String,
    #[arg(
        long = "authrpc.addr",
        default_value = "127.0.0.1",
        value_name = "ADDRESS",
        help = "Listening address for the authenticated rpc server.",
        help_heading = "RPC options"
    )]
    pub authrpc_addr: String,
    #[arg(
        long = "authrpc.port",
        default_value = "8551",
        value_name = "PORT",
        help = "Listening port for the authenticated rpc server.",
        help_heading = "RPC options"
    )]
    pub authrpc_port: String,
    #[arg(
        long = "authrpc.jwtsecret",
        default_value = "jwt.hex",
        value_name = "JWTSECRET_PATH",
        help = "Receives the jwt secret used for authenticated rpc requests.",
        help_heading = "RPC options"
    )]
    pub authrpc_jwtsecret: String,
    #[arg(long = "p2p.disabled", default_value = "false", value_name = "P2P_DISABLED", action = ArgAction::SetTrue, help_heading = "P2P options")]
    pub p2p_disabled: bool,
    #[arg(
        long = "p2p.addr",
        value_name = "ADDRESS",
        help = "Listening address for the P2P protocol.",
        help_heading = "P2P options"
    )]
    pub p2p_addr: Option<String>,
    #[arg(
        long = "p2p.port",
        default_value = "30303",
        value_name = "PORT",
        help = "TCP port for the P2P protocol.",
        help_heading = "P2P options"
    )]
    pub p2p_port: String,
    #[arg(
        long = "discovery.port",
        default_value = "30303",
        value_name = "PORT",
        help = "UDP port for P2P discovery.",
        help_heading = "P2P options"
    )]
    pub discovery_port: String,
    #[arg(
        long = "p2p.tx-broadcasting-interval",
        default_value_t = BROADCAST_INTERVAL_MS,
        value_name = "INTERVAL_MS",
        help = "Transaction Broadcasting Time Interval (ms) for batching transactions before broadcasting them.",
        help_heading = "P2P options"
    )]
    pub tx_broadcasting_time_interval: u64,
    #[arg(
        long = "p2p.target-peers",
        default_value_t = TARGET_PEERS,
        value_name = "MAX_PEERS",
        help = "Max amount of connected peers.",
        help_heading = "P2P options"
    )]
    pub target_peers: usize,
    #[arg(
        long = "p2p.lookup-interval",
        default_value_t = INITIAL_LOOKUP_INTERVAL_MS,
        value_name = "INITIAL_LOOKUP_INTERVAL",
        help = "Initial Lookup Time Interval (ms) to trigger each Discovery lookup message and RLPx connection attempt.",
        help_heading = "P2P options"
    )]
    pub lookup_interval: f64,
    #[arg(
        long = "builder.extra-data",
        default_value = get_minimal_client_version(),
        value_name = "EXTRA_DATA",
        help = "Block extra data message.",
        help_heading = "Block building options"
    )]
    pub extra_data: String,
    #[arg(
        long = "builder.gas-limit",
        default_value_t = DEFAULT_BUILDER_GAS_CEIL,
        value_name = "GAS_LIMIT",
        help = "Target block gas limit.",
        help_heading = "Block building options"
    )]
    pub gas_limit: u64,
    #[arg(
        long = "builder.max-blobs",
        value_name = "MAX_BLOBS",
        help = "EIP-7872: Maximum blobs per block for local building. Minimum of 1. Defaults to protocol max.",
        help_heading = "Block building options",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub max_blobs_per_block: Option<u32>,
    #[arg(
        long = "precompute-witnesses",
        action = ArgAction::SetTrue,
        default_value = "false",
        help = "Once synced, computes execution witnesses upon receiving newPayload messages and stores them in local storage",
        help_heading = "Node options"
    )]
    pub precompute_witnesses: bool,
}

impl Options {
    pub fn default_l1() -> Self {
        Self {
            network: Some(Network::LocalDevnet),
            datadir: DB_ETHREX_DEV_L1.into(),
            dev: true,
            http_addr: "0.0.0.0".to_string(),
            http_port: "8545".to_string(),
            authrpc_port: "8551".to_string(),
            metrics_port: "9090".to_string(),
            authrpc_addr: "localhost".to_string(),
            authrpc_jwtsecret: "jwt.hex".to_string(),
            p2p_port: "30303".into(),
            discovery_port: "30303".into(),
            mempool_max_size: 10_000,
            ..Default::default()
        }
    }

    #[cfg(feature = "l2")]
    pub fn default_l2() -> Self {
        Self {
            network: Some(Network::LocalDevnetL2),
            datadir: DB_ETHREX_DEV_L2.into(),
            metrics_port: "3702".into(),
            metrics_enabled: true,
            dev: true,
            http_addr: "0.0.0.0".into(),
            http_port: "1729".into(),
            authrpc_addr: "localhost".into(),
            authrpc_port: "8551".into(),
            authrpc_jwtsecret: "jwt.hex".into(),
            p2p_port: "30303".into(),
            discovery_port: "30303".into(),
            mempool_max_size: 10_000,
            ..Default::default()
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            http_addr: Default::default(),
            http_port: Default::default(),
            ws_enabled: false,
            ws_addr: Default::default(),
            ws_port: Default::default(),
            log_level: Level::INFO,
            log_color: Default::default(),
            log_dir: None,
            authrpc_addr: Default::default(),
            authrpc_port: Default::default(),
            authrpc_jwtsecret: Default::default(),
            p2p_disabled: Default::default(),
            p2p_addr: None,
            p2p_port: Default::default(),
            discovery_port: Default::default(),
            network: Default::default(),
            bootnodes: Default::default(),
            datadir: Default::default(),
            syncmode: Default::default(),
            metrics_addr: "0.0.0.0".to_owned(),
            metrics_port: Default::default(),
            metrics_enabled: Default::default(),
            dev: Default::default(),
            force: false,
            mempool_max_size: Default::default(),
            tx_broadcasting_time_interval: Default::default(),
            target_peers: Default::default(),
            lookup_interval: Default::default(),
            extra_data: get_minimal_client_version(),
            gas_limit: DEFAULT_BUILDER_GAS_CEIL,
            max_blobs_per_block: None,
            precompute_witnesses: false,
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(ClapSubcommand)]
pub enum Subcommand {
    #[command(name = "removedb", about = "Remove the database")]
    RemoveDB {
        #[arg(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value = default_datadir().into_os_string(), required = false)]
        datadir: PathBuf,
        #[arg(long = "force", help = "Force remove the database without confirmation", action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    #[command(name = "import", about = "Import blocks to the database")]
    Import {
        #[arg(
            required = true,
            value_name = "FILE_PATH/FOLDER",
            help = "Path to a RLP chain file or a folder containing files with individual Blocks"
        )]
        path: String,
        #[arg(long = "removedb", action = ArgAction::SetTrue)]
        removedb: bool,
        #[arg(long, action = ArgAction::SetTrue)]
        l2: bool,
    },
    #[command(
        name = "import-bench",
        about = "Import blocks to the database for benchmarking"
    )]
    ImportBench {
        #[arg(
            required = true,
            value_name = "FILE_PATH/FOLDER",
            help = "Path to a RLP chain file or a folder containing files with individual Blocks"
        )]
        path: String,
        #[arg(long = "removedb", action = ArgAction::SetTrue)]
        removedb: bool,
        #[arg(long, action = ArgAction::SetTrue)]
        l2: bool,
    },
    #[command(
        name = "export",
        about = "Export blocks in the current chain into a file in rlp encoding"
    )]
    Export {
        #[arg(
            required = true,
            value_name = "FILE_PATH",
            help = "Path to the file where the rlp blocks will be written to"
        )]
        path: String,
        #[arg(
            long = "first",
            value_name = "NUMBER",
            help = "First block number to export"
        )]
        first: Option<u64>,
        #[arg(
            long = "last",
            value_name = "NUMBER",
            help = "Last block number to export"
        )]
        last: Option<u64>,
    },
    #[command(
        name = "export-jsonl",
        about = "Export blocks as engine API JSONL requests (newPayloadV3 + forkchoiceUpdatedV3)"
    )]
    ExportJsonl {
        #[arg(
            required = true,
            value_name = "OUTPUT_PREFIX",
            help = "Output file prefix: creates <PREFIX>-payloads.jsonl and <PREFIX>-fcus.jsonl"
        )]
        prefix: String,
        #[arg(
            long = "first",
            value_name = "NUMBER",
            help = "First block number to export"
        )]
        first: Option<u64>,
        #[arg(
            long = "last",
            value_name = "NUMBER",
            help = "Last block number to export"
        )]
        last: Option<u64>,
    },
    #[command(
        name = "truncate",
        about = "Remove the latest N blocks from the chain, optionally exporting them first"
    )]
    Truncate {
        #[arg(
            long = "count",
            required = true,
            value_name = "N",
            help = "Number of latest blocks to remove"
        )]
        count: u64,
        #[arg(
            long = "export",
            value_name = "FILE_PATH",
            help = "Export removed blocks to RLP file before truncation"
        )]
        export: Option<String>,
        #[arg(
            long = "export-jsonl",
            value_name = "OUTPUT_PREFIX",
            help = "Export removed blocks as engine API JSONL before truncation (creates <PREFIX>-payloads.jsonl and <PREFIX>-fcus.jsonl)"
        )]
        export_jsonl: Option<String>,
    },
    #[command(
        name = "compute-state-root",
        about = "Compute the state root from a genesis file"
    )]
    ComputeStateRoot {
        #[arg(
            required = true,
            long = "path",
            value_name = "GENESIS_FILE_PATH",
            help = "Path to the genesis json file"
        )]
        genesis_path: PathBuf,
    },
    #[command(name = "repl", about = "Interactive REPL for Ethereum JSON-RPC")]
    Repl {
        /// JSON-RPC endpoint URL
        #[arg(short = 'e', long, default_value = "http://localhost:8545")]
        endpoint: String,

        /// Path to command history file
        #[arg(long, default_value = "~/.ethrex/history")]
        history_file: String,

        /// Execute a single command and exit
        #[arg(short = 'x', long)]
        execute: Option<String>,
    },
    #[cfg(feature = "l2")]
    #[command(name = "l2")]
    L2(crate::l2::L2Command),
}

impl Subcommand {
    pub async fn run(self, opts: &Options) -> eyre::Result<()> {
        // L2 has its own init_tracing because of the ethrex monitor
        let _guard = match &self {
            #[cfg(feature = "l2")]
            Self::L2(_) => None,
            _ => {
                let (_, guard) = init_tracing(opts);
                guard
            }
        };

        match self {
            Subcommand::RemoveDB { datadir, force } => {
                remove_db(&datadir, force);
            }
            Subcommand::Import { path, removedb, l2 } => {
                if removedb {
                    remove_db(&opts.datadir.clone(), opts.force);
                }

                let network = get_network(opts);
                let genesis = network.get_genesis()?;
                let blockchain_type = if l2 {
                    BlockchainType::L2(L2Config::default())
                } else {
                    BlockchainType::L1
                };
                import_blocks(
                    &path,
                    &opts.datadir,
                    genesis,
                    BlockchainOptions {
                        max_mempool_size: opts.mempool_max_size,
                        r#type: blockchain_type,
                        ..Default::default()
                    },
                )
                .await?;
            }
            Subcommand::ImportBench { path, removedb, l2 } => {
                if removedb {
                    remove_db(&opts.datadir.clone(), opts.force);
                }
                info!("ethrex version: {}", get_client_version());

                let network = get_network(opts);
                let genesis = network.get_genesis()?;
                let blockchain_type = if l2 {
                    BlockchainType::L2(L2Config::default())
                } else {
                    BlockchainType::L1
                };
                import_blocks_bench(
                    &path,
                    &opts.datadir,
                    genesis,
                    BlockchainOptions {
                        r#type: blockchain_type,
                        perf_logs_enabled: true,
                        ..Default::default()
                    },
                )
                .await?;
            }
            Subcommand::Export { path, first, last } => {
                export_blocks(&path, &opts.datadir, first, last).await
            }
            Subcommand::ExportJsonl {
                prefix,
                first,
                last,
            } => {
                export_blocks_jsonl(&prefix, &opts.datadir, first, last).await?;
            }
            Subcommand::Truncate {
                count,
                export,
                export_jsonl,
            } => {
                truncate_blocks(&opts.datadir, count, export.as_deref(), export_jsonl.as_deref())
                    .await?;
            }
            Subcommand::ComputeStateRoot { genesis_path } => {
                let genesis = Network::from(genesis_path).get_genesis()?;
                let state_root = genesis.compute_state_root();
                println!("{state_root:#x}");
            }
            Subcommand::Repl {
                endpoint,
                history_file,
                execute,
            } => {
                ethrex_repl::run(endpoint, history_file, execute).await;
            }
            #[cfg(feature = "l2")]
            Subcommand::L2(command) => command.run().await?,
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum LogColor {
    #[default]
    Auto,
    Always,
    Never,
}

impl Display for LogColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogColor::Auto => write!(f, "auto"),
            LogColor::Always => write!(f, "always"),
            LogColor::Never => write!(f, "never"),
        }
    }
}

impl FromStr for LogColor {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(LogColor::Auto),
            "always" => Ok(LogColor::Always),
            "never" => Ok(LogColor::Never),
            _ => Err(format!(
                "Invalid log color '{}'. Expected: auto, always, or never",
                s
            )),
        }
    }
}

pub fn remove_db(datadir: &Path, force: bool) {
    init_datadir(datadir);

    if datadir.exists() {
        if force {
            std::fs::remove_dir_all(datadir).expect("Failed to remove data directory");
            info!("Database removed successfully.");
        } else {
            print!("Are you sure you want to remove the database? (y/n): ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();

            if input.trim().eq_ignore_ascii_case("y") {
                std::fs::remove_dir_all(datadir).expect("Failed to remove data directory");
                println!("Database removed successfully.");
            } else {
                println!("Operation canceled.");
            }
        }
    } else {
        warn!("Data directory does not exist: {datadir:?}");
    }
}

pub async fn import_blocks(
    path: &str,
    datadir: &Path,
    genesis: Genesis,
    blockchain_opts: BlockchainOptions,
) -> Result<(), ChainError> {
    const IMPORT_BATCH_SIZE: usize = 1024;
    // This value is higher than the spec (128) as the latter block's state nodes will be kept in memory and not committed when using rocksdb
    // This means we need to run some extra-blocks to ensure we commit their state and don't need to regenerate the full block range upon node restart
    const MIN_FULL_BLOCKS: usize = 132;
    let start_time = Instant::now();
    init_datadir(datadir);
    let store = init_store(datadir, genesis).await?;
    let blockchain = init_blockchain(store.clone(), blockchain_opts);
    let path_metadata = metadata(path).expect("Failed to read path");

    // If it's an .rlp file it will be just one chain, but if it's a directory there can be multiple chains.
    let chains: Vec<Vec<Block>> = if path_metadata.is_dir() {
        info!(path = %path, "Importing blocks from directory");
        let mut entries: Vec<_> = read_dir(path)
            .expect("Failed to read blocks directory")
            .map(|res| res.expect("Failed to open file in directory").path())
            .collect();

        // Sort entries to process files in order (e.g., 1.rlp, 2.rlp, ...)
        entries.sort();

        entries
            .iter()
            .map(|entry| {
                let path_str = entry.to_str().expect("Couldn't convert path to string");
                info!(path = %path_str, "Importing blocks from file");
                utils::read_chain_file(path_str)
            })
            .collect()
    } else {
        info!(path = %path, "Importing blocks from file");
        vec![utils::read_chain_file(path)]
    };

    let mut total_blocks_imported = 0;
    for blocks in chains {
        let mut block_batch = vec![];
        let size = blocks.len();
        let mut numbers_and_hashes = blocks
            .iter()
            .map(|b| (b.header.number, b.hash()))
            .collect::<Vec<_>>();
        // Execute block by block
        let mut last_progress_log = Instant::now();
        for (index, block) in blocks.into_iter().enumerate() {
            let hash = block.hash();
            let number = block.header.number;

            // Log progress every 10 seconds
            if last_progress_log.elapsed() >= Duration::from_secs(10) {
                let processed = index + 1;
                let percent = (((processed as f64 / size as f64) * 100.0) * 10.0).round() / 10.0;
                info!(processed, total = size, percent, "Import progress");
                last_progress_log = Instant::now();
            }

            // Check if the block is already in the blockchain, if it is do nothing, if not add it
            let block_number = store.get_block_number(hash).await.map_err(|_e| {
                ChainError::Custom(String::from(
                    "Couldn't check if block is already in the blockchain",
                ))
            })?;

            if block_number.is_some() {
                info!("Block {} is already in the blockchain", block.hash());
                continue;
            }

            validate_block_body(&block.header, &block.body)
                .map_err(InvalidBlockError::InvalidBody)?;

            if index + MIN_FULL_BLOCKS < size {
                block_batch.push(block);
                if block_batch.len() >= IMPORT_BATCH_SIZE || index + MIN_FULL_BLOCKS + 1 == size {
                    blockchain
                        .add_blocks_in_batch(mem::take(&mut block_batch), CancellationToken::new())
                        .await
                        .map_err(|(err, _)| err)?;
                }
            } else {
                // We need to have the state of the latest 128 blocks
                blockchain
                .add_block_pipeline(block, None)
                .inspect_err(|err| match err {
                    // Block number 1's parent not found, the chain must not belong to the same network as the genesis file
                    ChainError::ParentNotFound if number == 1 => warn!("The chain file is not compatible with the genesis file. Are you sure you selected the correct network?"),
                    _ => warn!("Failed to add block {number} with hash {hash:#x}"),
                })?;
            }
        }

        // Make head canonical and label all special blocks correctly.
        if let Some((head_number, head_hash)) = numbers_and_hashes.pop() {
            store
                .forkchoice_update(
                    numbers_and_hashes,
                    head_number,
                    head_hash,
                    Some(head_number),
                    Some(head_number),
                )
                .await?;
        }

        total_blocks_imported += size;
    }

    let total_duration = start_time.elapsed();
    info!(
        blocks = total_blocks_imported,
        seconds = total_duration.as_secs_f64(),
        "Import completed"
    );
    Ok(())
}

pub async fn import_blocks_bench(
    path: &str,
    datadir: &Path,
    genesis: Genesis,
    blockchain_opts: BlockchainOptions,
) -> Result<(), ChainError> {
    let start_time = Instant::now();
    init_datadir(datadir);
    let store = init_store(datadir, genesis).await?;
    let blockchain = init_blockchain(store.clone(), blockchain_opts);
    regenerate_head_state(&store, &blockchain).await.unwrap();
    let path_metadata = metadata(path).expect("Failed to read path");

    // If it's an .rlp file it will be just one chain, but if it's a directory there can be multiple chains.
    let chains: Vec<Vec<Block>> = if path_metadata.is_dir() {
        info!(path = %path, "Importing blocks from directory");
        let mut entries: Vec<_> = read_dir(path)
            .expect("Failed to read blocks directory")
            .map(|res| res.expect("Failed to open file in directory").path())
            .collect();

        // Sort entries to process files in order (e.g., 1.rlp, 2.rlp, ...)
        entries.sort();

        entries
            .iter()
            .map(|entry| {
                let path_str = entry.to_str().expect("Couldn't convert path to string");
                info!(path = %path_str, "Importing blocks from file");
                utils::read_chain_file(path_str)
            })
            .collect()
    } else {
        info!(path = %path, "Importing blocks from file");
        vec![utils::read_chain_file(path)]
    };

    let mut total_blocks_imported = 0;
    for blocks in chains {
        let size = blocks.len();
        let mut numbers_and_hashes = blocks
            .iter()
            .map(|b| (b.header.number, b.hash()))
            .collect::<Vec<_>>();
        // Execute block by block
        let mut last_progress_log = Instant::now();
        for (index, block) in blocks.into_iter().enumerate() {
            let hash = block.hash();
            let number = block.header.number;

            // Log progress every 10 seconds
            if last_progress_log.elapsed() >= Duration::from_secs(10) {
                let processed = index + 1;
                let percent = (((processed as f64 / size as f64) * 100.0) * 10.0).round() / 10.0;
                info!(processed, total = size, percent, "Import progress");
                last_progress_log = Instant::now();
            }

            // Check if the block is already in the blockchain, if it is do nothing, if not add it
            let block_number = store.get_block_number(hash).await.map_err(|_e| {
                ChainError::Custom(String::from(
                    "Couldn't check if block is already in the blockchain",
                ))
            })?;

            if block_number.is_some() {
                info!("Block {} is already in the blockchain", block.hash());
                continue;
            }

            validate_block_body(&block.header, &block.body)
                .map_err(InvalidBlockError::InvalidBody)?;

            blockchain
                .add_block_pipeline(block, None)
                .inspect_err(|err| match err {
                    // Block number 1's parent not found, the chain must not belong to the same network as the genesis file
                    ChainError::ParentNotFound if number == 1 => warn!("The chain file is not compatible with the genesis file. Are you sure you selected the correct network?"),
                    _ => warn!("Failed to add block {number} with hash {hash:#x}"),
                })?;

            // TODO: replace this
            // This sleep is because we have a background process writing to disk the last layer
            // And until it's done we can't execute the new block
            // Because this wants to compare against running a real node in terms of reported performance
            // It takes less than 500ms, so this is good enough, but we should report the performance
            // without taking into account that wait.
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Make head canonical and label all special blocks correctly.
        if let Some((head_number, head_hash)) = numbers_and_hashes.pop() {
            store
                .forkchoice_update(
                    numbers_and_hashes,
                    head_number,
                    head_hash,
                    Some(head_number),
                    Some(head_number),
                )
                .await?;
        }

        total_blocks_imported += size;
    }

    let total_duration = start_time.elapsed();
    info!(
        blocks = total_blocks_imported,
        seconds = total_duration.as_secs_f64(),
        "Import completed"
    );
    Ok(())
}

pub async fn export_blocks(
    path: &str,
    datadir: &Path,
    first_number: Option<u64>,
    last_number: Option<u64>,
) {
    init_datadir(datadir);
    let store = match load_store(datadir).await {
        Err(err) => {
            error!("Failed to load Store due to: {err}");
            return;
        }
        Ok(store) => store,
    };
    export_blocks_with_store(path, &store, first_number, last_number).await;
}

pub async fn export_blocks_with_store(
    path: &str,
    store: &Store,
    first_number: Option<u64>,
    last_number: Option<u64>,
) {
    let start = first_number.unwrap_or_default();

    // If we have no latest block then we don't have any blocks to export
    let latest_number = match store.get_latest_block_number().await {
        Ok(number) => number,
        Err(StoreError::MissingLatestBlockNumber) => {
            warn!("No blocks in the current chain, nothing to export!");
            return;
        }
        Err(_) => panic!("Internal DB Error"),
    };

    // Get the earliest available block number to detect pruned blocks
    let earliest_number = match store.get_earliest_block_number().await {
        Ok(number) => number,
        Err(StoreError::MissingEarliestBlockNumber) => {
            // No earliest block set means everything from genesis is available
            0
        }
        Err(err) => {
            error!("Failed to get earliest block number: {err}");
            return;
        }
    };

    // Check that the requested range doesn't exceed our current chain length
    if last_number.is_some_and(|number| number > latest_number) {
        warn!(
            "The requested block range exceeds the current amount of blocks in the chain {latest_number}"
        );
        return;
    }
    let end = last_number.unwrap_or(latest_number);

    // Check if start is before earliest available block (pruned data)
    let adjusted_start = if start < earliest_number {
        warn!(
            requested_start = start,
            earliest_available = earliest_number,
            "Requested start block has been pruned, adjusting to earliest available block"
        );
        earliest_number
    } else {
        start
    };

    // Check that the requested range makes sense
    if adjusted_start > end {
        warn!("Cannot export block range [{adjusted_start}..{end}], please input a valid range");
        return;
    }

    // Fetch blocks from the store and export them to the file
    let mut file = File::create(path).expect("Failed to open file");
    let mut buffer = vec![];
    let mut last_output = Instant::now();
    let mut exported_count: u64 = 0;

    // Denominator for percent completed; avoid division by zero
    let denom = end.saturating_sub(adjusted_start) + 1;
    for n in adjusted_start..=end {
        let block = match store.get_block_by_number(n).await {
            Ok(Some(block)) => block,
            Ok(None) => {
                error!(
                    block_number = n,
                    earliest_available = earliest_number,
                    latest_available = latest_number,
                    "Block is missing within the available range - this indicates database corruption or unexpected state"
                );
                return;
            }
            Err(err) => {
                error!(block_number = n, error = %err, "Failed to read block from DB");
                return;
            }
        };
        block.encode(&mut buffer);
        file.write_all(&buffer).expect("Failed to write to file");
        buffer.clear();
        exported_count += 1;

        // Exporting the whole chain can take a while, so we need to show some output in the meantime
        if last_output.elapsed() > Duration::from_secs(5) {
            let percent = (exported_count * 100) / denom;
            info!(
                current_block = n,
                end_block = end,
                percent_complete = percent,
                blocks_exported = exported_count,
                "Exporting blocks"
            );
            last_output = Instant::now();
        }
    }

    info!(
        blocks_exported = exported_count,
        blocks_requested = end.saturating_sub(start) + 1,
        path = %path,
        "Exported blocks to file"
    );
}

pub async fn truncate_blocks(
    datadir: &Path,
    count: u64,
    export_path: Option<&str>,
    export_jsonl_prefix: Option<&str>,
) -> Result<(), eyre::Error> {
    if count == 0 {
        info!("Nothing to truncate (count = 0)");
        return Ok(());
    }

    init_datadir(datadir);
    let store = load_store(datadir).await?;

    let latest = match store.get_latest_block_number().await {
        Ok(number) => number,
        Err(StoreError::MissingLatestBlockNumber) => {
            warn!("No blocks in the current chain, nothing to truncate");
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    // Compute from_block, clamping to keep genesis
    let from_block = latest.saturating_sub(count - 1).max(1);

    info!(
        latest_block = latest,
        from_block = from_block,
        blocks_to_remove = latest - from_block + 1,
        "Truncating blocks"
    );

    // Export first if requested (reuse the already-open store to avoid RocksDB lock conflicts)
    if let Some(path) = export_path {
        info!(path = path, "Exporting blocks (RLP) before truncation");
        export_blocks_with_store(path, &store, Some(from_block), Some(latest)).await;
    }
    if let Some(prefix) = export_jsonl_prefix {
        info!(prefix = prefix, "Exporting blocks (JSONL) before truncation");
        export_blocks_jsonl_with_store(prefix, &store, Some(from_block), Some(latest)).await?;
    }

    let removed = store.truncate_from(from_block).await?;

    let new_latest = from_block - 1;
    info!(
        blocks_removed = removed,
        new_latest = new_latest,
        "Truncation complete"
    );

    Ok(())
}

pub async fn export_blocks_jsonl(
    prefix: &str,
    datadir: &Path,
    first_number: Option<u64>,
    last_number: Option<u64>,
) -> Result<(), eyre::Error> {
    init_datadir(datadir);
    let store = load_store(datadir).await?;
    export_blocks_jsonl_with_store(prefix, &store, first_number, last_number).await
}

pub async fn export_blocks_jsonl_with_store(
    prefix: &str,
    store: &Store,
    first_number: Option<u64>,
    last_number: Option<u64>,
) -> Result<(), eyre::Error> {
    let latest_number = match store.get_latest_block_number().await {
        Ok(number) => number,
        Err(StoreError::MissingLatestBlockNumber) => {
            warn!("No blocks in the current chain, nothing to export");
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    let start = first_number.unwrap_or(1); // Skip genesis for engine API replay
    let end = last_number.unwrap_or(latest_number);

    if end > latest_number {
        warn!("Requested block range exceeds current chain length {latest_number}");
        return Ok(());
    }
    if start > end {
        warn!("Cannot export block range [{start}..{end}], please input a valid range");
        return Ok(());
    }

    let payloads_path = format!("{prefix}-payloads.jsonl");
    let fcus_path = format!("{prefix}-fcus.jsonl");

    let payloads_file = BufWriter::new(File::create(&payloads_path)?);
    let fcus_file = BufWriter::new(File::create(&fcus_path)?);

    let mut payloads_writer = payloads_file;
    let mut fcus_writer = fcus_file;

    let mut exported_count: u64 = 0;
    let mut last_output = Instant::now();
    let denom = end.saturating_sub(start) + 1;

    for n in start..=end {
        let block = match store.get_block_by_number(n).await {
            Ok(Some(block)) => block,
            Ok(None) => {
                warn!(block_number = n, "Block is missing, skipping");
                continue;
            }
            Err(err) => {
                error!(block_number = n, error = %err, "Failed to read block from DB");
                return Err(err.into());
            }
        };

        let block_hash = block.hash();
        let block_number = block.header.number;
        let parent_beacon_block_root = block.header.parent_beacon_block_root.unwrap_or_default();

        // Collect blob versioned hashes from EIP-4844 transactions
        let blob_versioned_hashes: Vec<ethrex_common::H256> = block
            .body
            .transactions
            .iter()
            .flat_map(|tx| tx.blob_versioned_hashes())
            .collect();

        let payload = ExecutionPayload::from_block(block, None);

        // engine_newPayloadV3 request
        let payload_request = serde_json::json!({
            "id": block_number,
            "jsonrpc": "2.0",
            "method": "engine_newPayloadV3",
            "params": [
                payload,
                blob_versioned_hashes,
                parent_beacon_block_root,
            ]
        });

        serde_json::to_writer(&mut payloads_writer, &payload_request)?;
        payloads_writer.write_all(b"\n")?;

        // engine_forkchoiceUpdatedV3 request
        let fcu_request = serde_json::json!({
            "id": block_number,
            "jsonrpc": "2.0",
            "method": "engine_forkchoiceUpdatedV3",
            "params": [
                ForkChoiceState {
                    head_block_hash: block_hash,
                    safe_block_hash: block_hash,
                    finalized_block_hash: block_hash,
                },
            ]
        });

        serde_json::to_writer(&mut fcus_writer, &fcu_request)?;
        fcus_writer.write_all(b"\n")?;

        exported_count += 1;

        if last_output.elapsed() > Duration::from_secs(5) {
            let percent = (exported_count * 100) / denom;
            info!(
                current_block = n,
                end_block = end,
                percent_complete = percent,
                blocks_exported = exported_count,
                "Exporting blocks (JSONL)"
            );
            last_output = Instant::now();
        }
    }

    payloads_writer.flush()?;
    fcus_writer.flush()?;

    info!(
        blocks_exported = exported_count,
        payloads_path = %payloads_path,
        fcus_path = %fcus_path,
        "Exported blocks to JSONL files"
    );

    Ok(())
}
