use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about = "Benchmark engine API: JSON-RPC vs REST/SSZ")]
pub struct Args {
    /// Base URL of the running ethrex authrpc port (e.g., http://localhost:8551).
    #[arg(long)]
    pub url: String,

    /// Path to the JWT secret hex file used by the ethrex instance.
    #[arg(long)]
    pub jwt_path: PathBuf,

    /// Number of iterations per (transport, workload) pair.
    #[arg(long, default_value_t = 100)]
    pub iterations: usize,

    /// Comma-separated transports to exercise.
    #[arg(long, value_delimiter = ',', default_value = "json,ssz")]
    pub transports: Vec<Transport>,

    /// Comma-separated workloads to exercise.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "newPayload,getPayload,blobs,bodies"
    )]
    pub workloads: Vec<Workload>,

    /// Write per-iteration data to this CSV file.
    #[arg(long)]
    pub csv_out: Option<PathBuf>,
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
