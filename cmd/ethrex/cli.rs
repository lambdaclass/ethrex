use std::{
    fs::{metadata, read_dir},
    io::{self, Write},
    path::{Path, PathBuf},
};

use clap::{ArgAction, Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::{error::ChainError, fork_choice::apply_fork_choice};
use ethrex_common::types::Genesis;
use ethrex_p2p::{sync::SyncMode, types::Node};
use ethrex_vm::EvmEngine;
use tracing::{info, warn, Level};

use crate::{
    initializers::{init_blockchain, init_store},
    networks::{Network, PublicNetwork},
    utils::{self, get_client_version, set_datadir},
    DEFAULT_DATADIR,
};

#[cfg(feature = "l2")]
use crate::l2;

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
        default_value_t = Network::default(),
        value_name = "GENESIS_FILE_PATH",
        help = "Receives a `Genesis` struct in json format. This is the only argument which is required. You can look at some example genesis files at `test_data/genesis*`.",
        long_help = "Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include holesky, sepolia, hoodi and mainnet.",
        help_heading = "Node options",
        env = "ETHREX_NETWORK",
        value_parser = clap::value_parser!(Network),
    )]
    pub network: Network,
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
        long_help = "If set it will be considered as `true`. The Binary has to be built with the `dev` feature enabled.",
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
    #[arg(long = "p2p.enabled", default_value = if cfg!(feature = "l2") { "false" } else { "true" }, value_name = "P2P_ENABLED", action = ArgAction::SetTrue, help_heading = "P2P options")]
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
            network: Network::PublicNetwork(PublicNetwork::Mainnet),
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
        #[arg(
            short,
            long = "limit",
            help = "Limit how many blocks to import if specified"
        )]
        limit: Option<usize>,
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
    #[cfg(feature = "l2")]
    #[command(subcommand)]
    L2(l2::Command),
}

impl Subcommand {
    pub async fn run(self, opts: &Options) -> eyre::Result<()> {
        match self {
            Subcommand::RemoveDB { datadir, force } => {
                remove_db(&datadir, force);
            }
            Subcommand::Import {
                path,
                removedb,
                limit,
            } => {
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

                let network = &opts.network;
                import_blocks(&path, &opts.datadir, network.get_genesis(), opts.evm, limit).await?;
            }
            Subcommand::ComputeStateRoot { genesis_path } => {
                let state_root = Network::from(genesis_path)
                    .get_genesis()
                    .compute_state_root();
                println!("{:#x}", state_root);
            }
            #[cfg(feature = "l2")]
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
    limit: Option<usize>,
) -> Result<(), ChainError> {
    let data_dir = set_datadir(data_dir);
    let store = init_store(&data_dir, genesis).await;
    let blockchain = init_blockchain(evm, store.clone());
    let path_metadata = metadata(path).expect("Failed to read path");
    let blocks = if path_metadata.is_dir() {
        let mut blocks = vec![];
        let dir_reader = read_dir(path).expect("Failed to read blocks directory");
        for file_res in dir_reader {
            let file = file_res.expect("Failed to open file in directory");
            let path = file.path();
            let s = path
                .to_str()
                .expect("Path could not be converted into string");
            blocks.push(utils::read_block_file(s));
        }
        blocks
    } else {
        info!("Importing blocks from chain file: {path}");
        utils::read_chain_file(path)
    };

    let size = blocks.len();

    let mut last_block = None;

    for (i, block) in blocks.into_iter().enumerate() {
        if let Some(limit) = limit {
            info!("Reached block limit {limit}.",);
            if i >= limit {
                break;
            }
        }

        let hash = block.hash();

        info!(
            "Adding block {} with hash {:#x} ({} / {}).",
            block.header.number, hash, i, size
        );
        // Check if the block is already in the blockchain, if it is do nothing, if not add it
        match store.get_block_number(hash).await {
            Ok(Some(_)) => {
                info!("Block {} is already in the blockchain", block.hash());
                last_block = Some(block);
                continue;
            }
            Ok(None) => {
                if let Err(error) = blockchain.add_block(&block).await {
                    warn!(
                        "Failed to add block {} with hash {:#x}",
                        block.header.number, hash
                    );
                    return Err(error);
                }
                last_block = Some(block);
            }
            Err(_) => {
                return Err(ChainError::Custom(String::from(
                    "Couldn't check if block is already in the blockchain",
                )));
            }
        }
    }

    if let Some(last_block) = last_block {
        let hash = last_block.hash();
        if let Err(error) = apply_fork_choice(&store, hash, hash, hash).await {
            warn!("Failed to apply fork choice: {}", error);
        }
    }
    info!("Added {size} blocks to blockchain");
    Ok(())
}
