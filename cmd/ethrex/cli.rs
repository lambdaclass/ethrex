use std::{
    fs::{File, metadata, read_dir},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use clap::{ArgAction, Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::{BlockchainType, error::ChainError};
use ethrex_common::types::{Block, Genesis};
use ethrex_p2p::sync::SyncMode;
use ethrex_p2p::types::Node;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::error::StoreError;
use tracing::{Level, info, warn};

use crate::{
    initializers::{get_network, init_blockchain, init_store, init_tracing, load_store},
    l2::{
        self,
        command::{DB_ETHREX_DEV_L1, DB_ETHREX_DEV_L2},
    },
    utils::{self, default_datadir, get_client_version, init_datadir},
};
use ethrex_config::networks::Network;

#[allow(clippy::upper_case_acronyms)]
#[derive(ClapParser)]
#[command(name="ethrex", author = "Lambdaclass", version=get_client_version(), about = "ethrex Execution client")]
pub struct CLI {
    #[command(flatten)]
    pub opts: Options,
    #[command(subcommand)]
    pub command: Option<Subcommand>,
}

#[derive(ClapParser, Debug)]
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
        default_value_t = default_datadir(),
        help = "Receives the name of the directory where the Database is located.",
        long_help = "If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.",
        help_heading = "Node options",
        env = "ETHREX_DATADIR"
    )]
    pub datadir: String,
    #[arg(
        long = "force",
        help = "Force remove the database",
        long_help = "Delete the database without confirmation.",
        action = clap::ArgAction::SetTrue,
        help_heading = "Node options"
    )]
    pub force: bool,
    #[arg(long = "syncmode", default_value = "full", value_name = "SYNC_MODE", value_parser = utils::parse_sync_mode, help = "The way in which the node will sync its state.", long_help = "Can be either \"full\" or \"snap\" with \"full\" as default value.", help_heading = "P2P options")]
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
        help = "The verbosity level used for logs.",
        long_help = "Possible values: info, debug, trace, warn, error",
        help_heading = "Node options")]
    pub log_level: Level,
    #[arg(
        long = "http.addr",
        default_value = "localhost",
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
        long = "authrpc.addr",
        default_value = "localhost",
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
    #[arg(long = "p2p.enabled", default_value = "true", value_name = "P2P_ENABLED", action = ArgAction::SetTrue, help_heading = "P2P options")]
    pub p2p_enabled: bool,
    #[arg(
        long = "p2p.addr",
        default_value = "0.0.0.0",
        value_name = "ADDRESS",
        help_heading = "P2P options"
    )]
    pub p2p_addr: String,
    #[arg(
        long = "p2p.port",
        default_value = "30303",
        value_name = "PORT",
        help_heading = "P2P options"
    )]
    pub p2p_port: String,
    #[arg(
        long = "discovery.addr",
        default_value = "0.0.0.0",
        value_name = "ADDRESS",
        help = "UDP address for P2P discovery.",
        help_heading = "P2P options"
    )]
    pub discovery_addr: String,
    #[arg(
        long = "discovery.port",
        default_value = "30303",
        value_name = "PORT",
        help = "UDP port for P2P discovery.",
        help_heading = "P2P options"
    )]
    pub discovery_port: String,
}

impl Options {
    pub fn default_l1() -> Self {
        Self {
            network: Some(Network::LocalDevnet),
            datadir: DB_ETHREX_DEV_L1.to_string(),
            dev: true,
            http_addr: "0.0.0.0".to_string(),
            http_port: "8545".to_string(),
            authrpc_port: "8551".to_string(),
            metrics_port: "9090".to_string(),
            authrpc_addr: "localhost".to_string(),
            authrpc_jwtsecret: "jwt.hex".to_string(),
            p2p_enabled: true,
            p2p_addr: "0.0.0.0".to_string(),
            p2p_port: "30303".to_string(),
            discovery_addr: "0.0.0.0".to_string(),
            discovery_port: "30303".to_string(),
            ..Default::default()
        }
    }

    pub fn default_l2() -> Self {
        Self {
            network: Some(Network::LocalDevnetL2),
            datadir: DB_ETHREX_DEV_L2.to_string(),
            metrics_port: "3702".into(),
            metrics_enabled: true,
            dev: true,
            http_addr: "0.0.0.0".into(),
            http_port: "1729".into(),
            authrpc_addr: "localhost".into(),
            authrpc_port: "8551".into(),
            authrpc_jwtsecret: "jwt.hex".into(),
            p2p_enabled: true,
            p2p_addr: "0.0.0.0".into(),
            p2p_port: "30303".into(),
            discovery_addr: "0.0.0.0".into(),
            discovery_port: "30303".into(),
            ..Default::default()
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            http_addr: Default::default(),
            http_port: Default::default(),
            log_level: Level::INFO,
            authrpc_addr: Default::default(),
            authrpc_port: Default::default(),
            authrpc_jwtsecret: Default::default(),
            p2p_enabled: Default::default(),
            p2p_addr: Default::default(),
            p2p_port: Default::default(),
            discovery_addr: Default::default(),
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
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(ClapSubcommand)]
pub enum Subcommand {
    #[command(name = "removedb", about = "Remove the database")]
    RemoveDB {
        #[arg(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value_t = default_datadir(), required = false)]
        datadir: String,
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
    #[command(name = "l2")]
    L2(l2::L2Command),
}

impl Subcommand {
    pub async fn run(self, opts: &Options) -> eyre::Result<()> {
        // L2 has its own init_tracing because of the ethrex monitor
        match self {
            Self::L2(_) => {}
            _ => init_tracing(opts),
        }
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
                    BlockchainType::L2
                } else {
                    BlockchainType::L1
                };
                import_blocks(&path, &opts.datadir, genesis, blockchain_type).await?;
            }
            Subcommand::Export { path, first, last } => {
                export_blocks(&path, &opts.datadir, first, last).await
            }
            Subcommand::ComputeStateRoot { genesis_path } => {
                let genesis = Network::from(genesis_path).get_genesis()?;
                let state_root = genesis.compute_state_root();
                println!("{state_root:#x}");
            }
            Subcommand::L2(command) => command.run().await?,
        }

        Ok(())
    }
}

pub fn remove_db(datadir: &str, force: bool) {
    let data_dir = init_datadir(datadir);
    let path = Path::new(&data_dir);

    if path.exists() {
        if force {
            std::fs::remove_dir_all(path).expect("Failed to remove data directory");
            info!("Database removed successfully.");
        } else {
            print!("Are you sure you want to remove the database? (y/n): ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();

            if input.trim().eq_ignore_ascii_case("y") {
                std::fs::remove_dir_all(path).expect("Failed to remove data directory");
                println!("Database removed successfully.");
            } else {
                println!("Operation canceled.");
            }
        }
    } else {
        warn!("Data directory does not exist: {}", data_dir);
    }
}

pub async fn import_blocks(
    path: &str,
    data_dir: &str,
    genesis: Genesis,
    blockchain_type: BlockchainType,
) -> Result<(), ChainError> {
    let start_time = Instant::now();
    let data_dir = init_datadir(data_dir);
    let store = init_store(&data_dir, genesis).await;
    let blockchain = init_blockchain(store.clone(), blockchain_type, false);
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
        for (index, block) in blocks.iter().enumerate() {
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

            blockchain
                .add_block(block)
                .await
                .inspect_err(|err| match err {
                    // Block number 1's parent not found, the chain must not belong to the same network as the genesis file
                    ChainError::ParentNotFound if number == 1 => warn!("The chain file is not compatible with the genesis file. Are you sure you selected the correct network?"),
                    _ => warn!("Failed to add block {number} with hash {hash:#x}"),
                })?;
        }

        // Make head canonical and label all special blocks correctly.
        if let Some((head_number, head_hash)) = numbers_and_hashes.pop() {
            store
                .forkchoice_update(
                    Some(numbers_and_hashes),
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
    data_dir: &str,
    first_number: Option<u64>,
    last_number: Option<u64>,
) {
    let data_dir = init_datadir(data_dir);
    let store = load_store(&data_dir).await;
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
    // Check that the requested range doesn't exceed our current chain length
    if last_number.is_some_and(|number| number > latest_number) {
        warn!(
            "The requested block range exceeds the current amount of blocks in the chain {latest_number}"
        );
        return;
    }
    let end = last_number.unwrap_or(latest_number);
    // Check that the requested range makes sense
    if start > end {
        warn!("Cannot export block range [{start}..{end}], please input a valid range");
        return;
    }
    // Fetch blocks from the store and export them to the file
    let mut file = File::create(path).expect("Failed to open file");
    let mut buffer = vec![];
    let mut last_output = Instant::now();
    // Denominator for percent completed; avoid division by zero
    let denom = end.saturating_sub(start) + 1;
    for n in start..=end {
        let block = store
            .get_block_by_number(n)
            .await
            .ok()
            .flatten()
            .expect("Failed to read block from DB");
        block.encode(&mut buffer);
        // Exporting the whole chain can take a while, so we need to show some output in the meantime
        if last_output.elapsed() > Duration::from_secs(5) {
            let completed = n.saturating_sub(start) + 1;
            let percent = (completed * 100) / denom;
            info!(n, end, percent, "Exporting blocks");
            last_output = Instant::now();
        }
        file.write_all(&buffer).expect("Failed to write to file");
        buffer.clear();
    }
    info!(blocks = end.saturating_sub(start), path = %path, "Exported blocks to file");
}
