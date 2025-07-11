use std::{
    fs::{File, metadata, read_dir},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use clap::{ArgAction, Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::{BlockchainType, error::ChainError};
use ethrex_common::types::{Block, Genesis};
use ethrex_p2p::{sync::SyncMode, types::Node};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::error::StoreError;
use ethrex_vm::EvmEngine;
use tracing::{Level, info, warn};

use crate::{
    DEFAULT_DATADIR,
    initializers::{get_network, init_blockchain, init_store, open_store},
    l2,
    networks::Network,
    utils::{self, get_client_version, set_datadir},
};

#[allow(clippy::upper_case_acronyms)]
#[derive(ClapParser)]
#[command(name="ethrex", author = "Lambdaclass", version=get_client_version(), about = "ethrex Execution client")]
pub struct CLI {
    #[command(flatten)]
    pub opts: Options,
    #[command(subcommand)]
    pub command: Option<Subcommand>,
}

#[derive(ClapParser)]
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
        default_value = DEFAULT_DATADIR,
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
        long = "evm",
        default_value_t = EvmEngine::default(),
        value_name = "EVM_BACKEND",
        help = "Has to be `levm` or `revm`",
        value_parser = utils::parse_evm_engine,
        help_heading = "Node options",
        env = "ETHREX_EVM")]
    pub evm: EvmEngine,
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
            evm: Default::default(),
            force: false,
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(ClapSubcommand)]
pub enum Subcommand {
    #[command(name = "removedb", about = "Remove the database")]
    RemoveDB {
        #[arg(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value = DEFAULT_DATADIR, required = false)]
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
    #[command(subcommand)]
    L2(l2::Command),
}

impl Subcommand {
    pub async fn run(self, opts: &Options) -> eyre::Result<()> {
        match self {
            Subcommand::RemoveDB { datadir, force } => {
                remove_db(&datadir, force);
            }
            Subcommand::Import { path, removedb, l2 } => {
                if removedb {
                    Box::pin(async {
                        Self::RemoveDB {
                            datadir: opts.datadir.clone(),
                            force: opts.force,
                        }
                        .run(opts)
                        .await
                    })
                    .await?;
                }

                let network = get_network(opts);
                let genesis = network.get_genesis()?;
                let blockchain_type = if l2 {
                    BlockchainType::L2
                } else {
                    BlockchainType::L1
                };
                import_blocks(&path, &opts.datadir, genesis, opts.evm, blockchain_type).await?;
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
    let data_dir = set_datadir(datadir);
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
    evm: EvmEngine,
    blockchain_type: BlockchainType,
) -> Result<(), ChainError> {
    let data_dir = set_datadir(data_dir);
    let store = init_store(&data_dir, genesis).await;
    let blockchain = init_blockchain(evm, store.clone(), blockchain_type);
    let path_metadata = metadata(path).expect("Failed to read path");

    // If it's an .rlp file it will be just one chain, but if it's a directory there can be multiple chains.
    let chains: Vec<Vec<Block>> = if path_metadata.is_dir() {
        info!("Importing blocks from directory: {path}");
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
                info!("Importing blocks from chain file: {path_str}");
                utils::read_chain_file(path_str)
            })
            .collect()
    } else {
        info!("Importing blocks from chain file: {path}");
        vec![utils::read_chain_file(path)]
    };

    for blocks in chains {
        let size = blocks.len();
        // Execute block by block
        for block in &blocks {
            let hash = block.hash();
            let number = block.header.number;
            info!("Adding block {number} with hash {hash:#x}.");
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
                .inspect_err(|_| warn!("Failed to add block {number} with hash {hash:#x}",))?;
        }

        _ = store
            .mark_chain_as_canonical(&blocks)
            .await
            .inspect_err(|error| warn!("Failed to apply fork choice: {}", error));

        // Make head canonical and label all special blocks correctly.
        if let Some(block) = blocks.last() {
            store
                .update_finalized_block_number(block.header.number)
                .await?;
            store.update_safe_block_number(block.header.number).await?;
            store
                .update_latest_block_number(block.header.number)
                .await?;
        }

        info!("Added {size} blocks to blockchain");
    }
    Ok(())
}

pub async fn export_blocks(
    path: &str,
    data_dir: &str,
    first_number: Option<u64>,
    last_number: Option<u64>,
) {
    let data_dir = set_datadir(data_dir);
    let store = open_store(&data_dir);
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
            info!("Exporting block {n}/{end}, {}% done", n * 100 / end);
            last_output = Instant::now();
        }
        file.write_all(&buffer).expect("Failed to write to file");
        buffer.clear();
    }
    info!("Exported {} blocks to file {path}", end - start);
}
