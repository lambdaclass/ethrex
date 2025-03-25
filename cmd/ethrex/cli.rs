use std::{
    fs::{metadata, read_dir},
    path::Path,
};

use clap::{ArgAction, Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_p2p::{sync::SyncMode, types::Node};
use ethrex_vm::EvmEngine;
use tracing::{info, warn, Level};

use crate::{
    initializers::{init_blockchain, init_store},
    utils::{self, set_datadir},
    DEFAULT_DATADIR,
};

#[cfg(feature = "l2")]
use secp256k1::SecretKey;

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

#[allow(clippy::upper_case_acronyms)]
#[derive(ClapParser)]
#[command(name="ethrex", author = "Lambdaclass", version=VERSION_STRING, about, about = "ethrex Execution client")]
pub struct CLI {
    #[clap(flatten)]
    pub opts: Options,
    #[cfg(feature = "l2")]
    #[clap(flatten)]
    pub l2_opts: L2Options,
    #[cfg(feature = "based")]
    #[clap(flatten)]
    pub based_opts: BasedOptions,
    #[command(subcommand)]
    pub command: Option<Subcommand>,
}

#[derive(ClapParser)]
pub struct Options {
    #[arg(
        long = "network",
        value_name = "GENESIS_FILE_PATH",
        help = "Receives a `Genesis` struct in json format. This is the only argument which is required. You can look at some example genesis files at `test_data/genesis*`.",
        long_help = "Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include holesky, sepolia and mekong.",
        help_heading = "Node options"
    )]
    pub network: Option<String>,
    #[arg(long = "bootnodes", value_parser = clap::value_parser!(Node), value_name = "BOOTNODE_LIST", value_delimiter = ',', num_args = 1.., help = "Comma separated enode URLs for P2P discovery bootstrap.", help_heading = "P2P options")]
    pub bootnodes: Vec<Node>,
    #[arg(
        long = "datadir",
        value_name = "DATABASE_DIRECTORY",
        help = "If the datadir is the word `memory`, ethrex will use the InMemory Engine",
        default_value = DEFAULT_DATADIR,
        help = "Receives the name of the directory where the Database is located.",
        long_help = "If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.",
        help_heading = "Node options"
    )]
    pub datadir: String,
    #[arg(long = "syncmode", default_value = "full", value_name = "SYNC_MODE", value_parser = utils::parse_sync_mode, help = "The way in which the node will sync its state.", long_help = "Can be either \"full\" or \"snap\" with \"full\" as default value.", help_heading = "P2P options")]
    pub syncmode: SyncMode,
    #[arg(
        long = "metrics.port",
        value_name = "PROMETHEUS_METRICS_PORT",
        help_heading = "Node options"
    )]
    pub metrics_port: Option<String>,
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
        default_value = "revm",
        value_name = "EVM_BACKEND",
        help = "Has to be `levm` or `revm`",
        value_parser = utils::parse_evm_engine,
        help_heading = "Node options"
    )]
    pub evm: EvmEngine,
    #[arg(long = "log.level", default_value_t = Level::INFO, value_name = "LOG_LEVEL", help = "The verbosity level used for logs.", long_help = "Possible values: info, debug, trace, warn, error",help_heading = "Node options")]
    pub log_level: Level,
    #[arg(
        long = "http.addr",
        default_value = "localhost",
        value_name = "ADDRESS",
        help = "Listening address for the http rpc server.",
        help_heading = "RPC options"
    )]
    pub http_addr: String,
    #[arg(
        long = "http.port",
        default_value = "8545",
        value_name = "PORT",
        help = "Listening port for the http rpc server.",
        help_heading = "RPC options"
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

#[cfg(feature = "l2")]
#[derive(ClapParser)]
pub struct L2Options {
    #[arg(
        long = "sponsorable-addresses",
        value_name = "SPONSORABLE_ADDRESSES_PATH",
        help = "Path to a file containing addresses of contracts to which ethrex_SendTransaction should sponsor txs",
        help_heading = "L2 options"
    )]
    pub sponsorable_addresses_file_path: Option<String>,
    #[arg(long, value_parser = utils::parse_private_key, env = "SPONSOR_PRIVATE_KEY", help = "The private key of ethrex L2 transactions sponsor.", help_heading = "L2 options")]
    pub sponsor_private_key: Option<SecretKey>,
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

                let network = opts
                    .network
                    .as_ref()
                    .expect("--network is required and it was not provided");

                let data_dir = set_datadir(&opts.datadir);

                let store = init_store(&data_dir, network);

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
