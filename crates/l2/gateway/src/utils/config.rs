use clap::Parser;
use reqwest::Url;
use std::{net::Ipv4Addr, path::PathBuf};
use tracing::level_filters::LevelFilter;

#[derive(Parser, Debug)]
#[command(version, about, name = "gateway")]
pub struct GatewayArgs {
    #[arg(long, value_name = "GENESIS_FILE_PATH")]
    pub network: PathBuf,
    /// The host to run the engine_ and eth_ RPC
    #[arg(long = "rpc.host", default_value_t = Ipv4Addr::UNSPECIFIED)]
    pub rpc_host: Ipv4Addr,
    /// The port to run the engine_ and eth_ RPC
    #[arg(long = "rpc.port", default_value_t = 9090)]
    pub rpc_port: u16,
    /// Url to a full node for syncing and eth_ fallback requests
    #[arg(
        long = "rpc.fallback_url",
        default_value = "https://base-sepolia-rpc.publicnode.com"
    )]
    pub rpc_fallback_url: Url,
    /// Url to the root peer gossip node
    #[arg(long = "gossip.root_peer_url")]
    pub gossip_root_peer_url: Option<Url>,
    /// Duration of a frag in ms
    #[arg(long = "sequencer.frag_duration_ms", default_value_t = 200)]
    pub frag_duration_ms: u64,
    /// Number of sims per loop
    #[arg(long = "sequencer.sim_threads", default_value_t = 5)]
    pub sim_threads: usize,
    /// Database location
    #[arg(long = "db.datadir")]
    pub db_datadir: PathBuf,
    /// Maximum number of cached accounts
    #[arg(long = "db.max_cached_accounts", default_value_t = 10_000)]
    pub max_cached_accounts: u64,
    /// Maximum number of cached storages
    #[arg(long = "db.max_cached_storages", default_value_t = 100_000)]
    pub max_cached_storages: u64,
    /// Test mode
    #[arg(long = "test")]
    pub test: bool,
    /// Enable DEBUG logging
    #[arg(long = "debug")]
    pub debug: bool,
    /// Enable TRACE logging
    #[arg(long = "trace")]
    pub trace: bool,
    /// Enable file logging
    #[arg(long = "log.enable_file_logging")]
    pub file_logging: bool,
    /// Prefix of log files
    #[arg(long = "log.prefix")]
    pub log_prefix: Option<String>,
    /// Maximum number of log files
    #[arg(long = "log.max_files", default_value_t = 30)]
    pub log_max_files: usize,
    /// Path for log files
    #[arg(long = "log.path", default_value = "/tmp")]
    pub log_path: PathBuf,
    /// Add additional filters for logging
    #[arg(long = "log.filters")]
    pub log_filters: Option<String>,
    /// If true will commit locally sequenced blocks to the db before getting payload from the engine api.
    #[arg(long = "sequencer.commit_sealed_frags_to_db", default_value_t = false)]
    pub commit_sealed_frags_to_db: bool,
}

#[derive(Debug, Clone)]
pub struct LoggingConfig {
    pub level: LevelFilter,
    pub enable_file_logging: bool,
    pub prefix: Option<String>,
    pub max_files: usize,
    pub path: PathBuf,
    pub filters: Option<String>,
}

impl From<&GatewayArgs> for LoggingConfig {
    fn from(args: &GatewayArgs) -> Self {
        Self {
            level: args
                .trace
                .then_some(LevelFilter::TRACE)
                .or(args.debug.then_some(LevelFilter::DEBUG))
                .unwrap_or(LevelFilter::INFO),
            enable_file_logging: args.file_logging,
            prefix: args.log_prefix.clone(),
            max_files: args.log_max_files,
            path: args.log_path.clone(),
            filters: args.log_filters.clone(),
        }
    }
}
