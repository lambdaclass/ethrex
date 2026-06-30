use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Benchmark engine API: JSON-RPC vs REST/SSZ, across all fork eras"
)]
pub struct Args {
    /// Benchmark an existing node at this authrpc URL instead of self-hosting
    /// per-fork devnets. The node's fork is auto-detected and only that fork's
    /// rows are produced. Requires --jwt-path.
    #[arg(long, requires = "jwt_path")]
    pub url: Option<String>,

    /// Path to the JWT secret hex file of the node given via --url.
    #[arg(long)]
    pub jwt_path: Option<PathBuf>,

    /// ethrex binary used to self-host the per-fork devnets (default mode,
    /// when --url is not given).
    #[arg(long, default_value = "target/release/ethrex")]
    pub ethrex_bin: PathBuf,

    /// authrpc port for self-hosted devnets (the http port is port+1). The
    /// harness refuses to start if something already listens here.
    #[arg(long, default_value_t = 18551)]
    pub devnet_port: u16,

    /// Scratch directory for self-hosted devnet datadirs; a fresh
    /// run-<timestamp> subdirectory is created inside (system temp dir by
    /// default) and removed after a successful run unless --keep-devnets.
    #[arg(long)]
    pub datadir_base: Option<PathBuf>,

    /// Keep devnet datadirs and node logs after the run.
    #[arg(long)]
    pub keep_devnets: bool,

    /// Number of iterations per (fork, workload, transport) cell.
    #[arg(long, default_value_t = 100)]
    pub iterations: usize,

    /// Comma-separated transports to exercise.
    #[arg(long, value_delimiter = ',', default_value = "json,ssz")]
    pub transports: Vec<Transport>,

    /// Comma-separated workloads to exercise. The blobs workload runs all
    /// three endpoint versions (v1/v2/v3) per fork.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "newPayload,getPayload,blobs,bodies"
    )]
    pub workloads: Vec<Workload>,

    /// Write per-iteration data to this CSV file.
    #[arg(long)]
    pub csv_out: Option<PathBuf>,

    /// Warmup iterations per cell, discarded from results. They absorb
    /// connection setup (TCP + h2c handshake) and server cold paths.
    #[arg(long, default_value_t = 3)]
    pub warmup: usize,

    /// Use this payloadId for the getPayload workload instead of acquiring one
    /// via forkchoiceUpdated (external mode only).
    #[arg(long)]
    pub payload_id: Option<String>,

    /// File of newline-separated 0x-prefixed versioned hashes for the blobs
    /// workload. Without it random hashes are used: every entry misses and the
    /// response carries no blob data (see the `hits` column).
    #[arg(long)]
    pub blob_hashes_file: Option<PathBuf>,

    /// First block number of the bodies range.
    #[arg(long, default_value_t = 1)]
    pub bodies_from: u64,

    /// Number of bodies to request. The REST/SSZ endpoint caps a single
    /// request at MAX_BODIES_PER_REQUEST (32); larger values make the SSZ
    /// side fail while JSON still answers.
    #[arg(long, default_value_t = 32)]
    pub bodies_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Transport {
    Json,
    Ssz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Workload {
    #[value(name = "newPayload")]
    NewPayload,
    #[value(name = "getPayload")]
    GetPayload,
    Blobs,
    Bodies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ForkArg {
    Paris,
    Shanghai,
    Cancun,
    Prague,
    Osaka,
    Amsterdam,
}

impl ForkArg {
    /// Every fork era, in activation order — the default-mode sweep.
    pub const ALL: [ForkArg; 6] = [
        ForkArg::Paris,
        ForkArg::Shanghai,
        ForkArg::Cancun,
        ForkArg::Prague,
        ForkArg::Osaka,
        ForkArg::Amsterdam,
    ];

    /// `Eth-Execution-Version` header value for the REST/SSZ API (also used as
    /// the display name).
    pub fn path(self) -> &'static str {
        match self {
            ForkArg::Paris => "paris",
            ForkArg::Shanghai => "shanghai",
            ForkArg::Cancun => "cancun",
            ForkArg::Prague => "prague",
            ForkArg::Osaka => "osaka",
            ForkArg::Amsterdam => "amsterdam",
        }
    }
}
