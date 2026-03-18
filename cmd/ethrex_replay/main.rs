mod replay;

use anyhow::{Context, Result};
use clap::Parser;
use ethrex_common::types::Genesis;
use ethrex_storage::{EngineType, Store};
use std::path::PathBuf;
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "ethrex-replay",
    about = "Replay blocks from a RocksDB store against a binary trie state"
)]
struct Args {
    /// Path to the ethrex RocksDB data directory.
    #[arg(long)]
    store_path: PathBuf,

    /// Path to the genesis JSON file.
    #[arg(long)]
    genesis_path: PathBuf,

    /// First block to replay (default: 1).
    #[arg(long, default_value_t = 1)]
    start_block: u64,

    /// Last block to replay (default: latest in store).
    #[arg(long)]
    end_block: Option<u64>,

    /// Log binary trie root every N blocks (default: 1000).
    #[arg(long, default_value_t = 1000)]
    log_interval: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    // Parse genesis JSON.
    let genesis_file = std::fs::File::open(&args.genesis_path).with_context(|| {
        format!(
            "Failed to open genesis file: {}",
            args.genesis_path.display()
        )
    })?;
    let genesis: Genesis =
        serde_json::from_reader(genesis_file).context("Failed to parse genesis JSON")?;

    // Open the existing RocksDB store (block source).
    info!("Opening store at {}", args.store_path.display());
    let store =
        Store::new(&args.store_path, EngineType::RocksDB).context("Failed to open store")?;
    store
        .load_initial_state()
        .await
        .context("Failed to load initial state")?;

    // Determine block range.
    let end_block = match args.end_block {
        Some(n) => n,
        None => store
            .get_latest_block_number()
            .await
            .context("Failed to get latest block number")?,
    };
    let start_block = args.start_block;

    info!("Replaying blocks {start_block}..={end_block}");

    // Initialize and run the replayer.
    let mut replayer = replay::BlockReplayer::new(genesis, store)?;
    replayer
        .replay(start_block, end_block, args.log_interval)
        .await?;

    Ok(())
}
