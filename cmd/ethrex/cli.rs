use std::{
    fs::{metadata, read_dir},
    path::Path,
};

use clap::{ArgAction, Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_p2p::{sync::SyncMode, types::Node};
use ethrex_vm::backends::EvmEngine;
use tracing::{info, warn, Level};

use crate::{
    initializers::{init_blockchain, init_store},
    utils::{self, set_datadir},
    DEFAULT_DATADIR,
};

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

#[allow(clippy::upper_case_acronyms)]
#[derive(ClapParser)]
#[command(name="ethrex", author, version=VERSION_STRING, about, long_about = None)]
pub struct CLI {
    #[clap(flatten)]
    pub opts: Options,
    #[cfg(feature = "based")]
    #[clap(flatten)]
    pub based_opts: BasedOptions,
    #[command(subcommand)]
    pub command: Option<Subcommand>,
}

#[derive(ClapParser)]
pub struct Options {
    #[arg(
        long = "http.addr",
        default_value = "localhost",
        value_name = "ADDRESS"
    )]
    pub http_addr: String,
    #[arg(long = "http.port", default_value = "8545", value_name = "PORT")]
    pub http_port: String,
    #[arg(long = "log.level", default_value_t = Level::INFO, value_name = "LOG_LEVEL")]
    pub log_level: Level,
    #[arg(
        long = "authrpc.addr",
        default_value = "localhost",
        value_name = "ADDRESS"
    )]
    pub authrpc_addr: String,
    #[arg(long = "authrpc.port", default_value = "8551", value_name = "PORT")]
    pub authrpc_port: String,
    #[arg(
        long = "authrpc.jwtsecret",
        default_value = "jwt.hex",
        value_name = "JWTSECRET_PATH"
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
        help_heading = "P2P options"
    )]
    pub discovery_addr: String,
    #[arg(
        long = "discovery.port",
        default_value = "30303",
        value_name = "PORT",
        help_heading = "P2P options"
    )]
    pub discovery_port: String,
    #[arg(long = "network", value_name = "GENESIS_FILE_PATH")]
    pub network: String,
    #[arg(long = "bootnodes", value_name = "BOOTNODE_LIST", value_delimiter = ',', num_args = 1..)]
    pub bootnodes: Vec<Node>,
    #[arg(
        long = "datadir",
        value_name = "DATABASE_DIRECTORY",
        help = "If the datadir is the word `memory`, ethrex will use the InMemory Engine",
        default_value = DEFAULT_DATADIR,
        required = false,
    )]
    pub datadir: String,
    #[arg(long = "syncmode", default_value = "full", value_name = "SYNC_MODE", value_parser = utils::parse_sync_mode)]
    pub syncmode: SyncMode,
    #[arg(long = "metrics.port", value_name = "PROMETHEUS_METRICS_PORT")]
    pub metrics_port: Option<String>,
    #[arg(
        long = "dev",
        help = "Used to create blocks without requiring a Consensus Client",
        action = ArgAction::SetTrue
    )]
    pub dev: bool,
    #[arg(
        long = "evm",
        default_value = "revm",
        value_name = "EVM_BACKEND",
        help = "Has to be `levm` or `revm`",
        value_parser = utils::parse_evm_engine
    )]
    pub evm: EvmEngine,
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
            metrics_port: Default::default(),
            dev: Default::default(),
            evm: Default::default(),
        }
    }
}

#[cfg(feature = "based")]
#[derive(ClapParser)]
pub struct BasedOptions {
    #[arg(
        long = "gateway.addr",
        default_value = "0.0.0.0",
        value_name = "GATEWAY_ADDRESS",
        help_heading = "Based options"
    )]
    pub gateway_addr: String,
    #[arg(
        long = "gateway.eth_port",
        default_value = "8546",
        value_name = "GATEWAY_ETH_PORT",
        help_heading = "Based options"
    )]
    pub gateway_eth_port: String,
    #[arg(
        long = "gateway.auth_port",
        default_value = "8553",
        value_name = "GATEWAY_AUTH_PORT",
        help_heading = "Based options"
    )]
    pub gateway_auth_port: String,
    #[arg(
        long = "gateway.jwtsecret",
        default_value = "jwt.hex",
        value_name = "GATEWAY_JWTSECRET_PATH",
        help_heading = "Based options"
    )]
    pub gateway_jwtsecret: String,
}

#[derive(ClapSubcommand)]
pub enum Subcommand {
    #[clap(name = "removedb", about = "Remove the database")]
    RemoveDB {
        #[clap(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value = DEFAULT_DATADIR, required = false)]
        datadir: String,
    },
    #[clap(name = "import", about = "Import blocks to the database")]
    Import {
        #[clap(
            required = true,
            value_name = "FILE_PATH/FOLDER",
            help = "Path to a RLP chain file or a folder containing files with individual Blocks"
        )]
        path: String,
        #[clap(long = "removedb", action = ArgAction::SetTrue)]
        removedb: bool,
    },
}

impl Subcommand {
    pub fn run(self, opts: &Options) -> eyre::Result<()> {
        match self {
            Subcommand::RemoveDB { datadir } => {
                let data_dir = set_datadir(&datadir);

                let path = Path::new(&data_dir);

                if path.exists() {
                    std::fs::remove_dir_all(path).expect("Failed to remove data directory");
                    info!("Successfully removed database at {data_dir}");
                } else {
                    warn!("Data directory does not exist: {data_dir}");
                }
            }
            Subcommand::Import { path, removedb } => {
                if removedb {
                    Self::RemoveDB {
                        datadir: opts.datadir.clone(),
                    }
                    .run(opts)?;
                }

                let store = init_store(&opts.datadir, &opts.network);

                let blockchain = init_blockchain(opts.evm, store);

                let path_metadata = metadata(&path).expect("Failed to read path");
                let blocks = if path_metadata.is_dir() {
                    let mut blocks = vec![];
                    let dir_reader = read_dir(&path).expect("Failed to read blocks directory");
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
                    utils::read_chain_file(&path)
                };
                blockchain.import_blocks(&blocks);
            }
        }
        Ok(())
    }
}
