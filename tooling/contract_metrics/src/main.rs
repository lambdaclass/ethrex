use clap::Parser;
use ethrex_common::{Address, types::TxKind};
use ethrex_storage::{EngineType, Store};
use std::{collections::HashMap, io::Write as _, path::PathBuf};

#[derive(Parser)]
#[command(about = "Extract contract deployment metrics from an ethrex mainnet database")]
struct Cli {
    /// Path to the ethrex data directory (must contain a RocksDB store)
    #[arg(long)]
    datadir: PathBuf,

    /// First block to analyse (inclusive). Defaults to `end - 10000`.
    #[arg(long)]
    start: Option<u64>,

    /// Last block to analyse (inclusive). Defaults to the latest stored block.
    #[arg(long)]
    end: Option<u64>,

    /// Convenience: analyse the last M blocks ending at `end`.
    /// Ignored if `--start` is given.
    #[arg(long, default_value = "10000")]
    blocks: u64,

    /// How many top-called contracts to print.
    #[arg(long, default_value = "20")]
    top_n: usize,

    /// Write per-block data as CSV to this file path.
    #[arg(long)]
    csv: Option<PathBuf>,
}

struct BlockStats {
    block_number: u64,
    total_txs: usize,
    /// Transactions with TxKind::Create (attempted deployments).
    create_txs: usize,
    /// CREATE transactions whose receipt shows success.
    successful_creates: usize,
    /// Total gas used in the block (from the last receipt's cumulative field).
    gas_used: u64,
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p / 100.0).round() as usize;
    sorted[idx]
}

fn print_summary(label: &str, mut values: Vec<u64>) {
    if values.is_empty() {
        println!("{label}: no data");
        return;
    }
    values.sort_unstable();
    let sum: u64 = values.iter().sum();
    let mean = sum as f64 / values.len() as f64;
    let min = values[0];
    let max = *values.last().unwrap();
    let p50 = percentile(&values, 50.0);
    let p95 = percentile(&values, 95.0);
    let p99 = percentile(&values, 99.0);
    println!("{label}: min={min}  mean={mean:.1}  p50={p50}  p95={p95}  p99={p99}  max={max}");
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    let store = Store::new(&cli.datadir, EngineType::RocksDB)?;

    let end = match cli.end {
        Some(n) => n,
        None => store.get_latest_block_number().await?,
    };
    let start = cli.start.unwrap_or_else(|| end.saturating_sub(cli.blocks));

    println!(
        "Scanning blocks {start}..={end} ({} blocks)",
        end - start + 1
    );

    let mut csv_writer: Option<std::fs::File> = if let Some(ref path) = cli.csv {
        let f = std::fs::File::create(path)?;
        Some(f)
    } else {
        None
    };

    if let Some(ref mut f) = csv_writer {
        writeln!(
            f,
            "block_number,total_txs,create_txs,successful_creates,gas_used"
        )?;
    }

    let mut stats: Vec<BlockStats> = Vec::with_capacity((end - start + 1) as usize);
    let mut call_counts: HashMap<Address, u64> = HashMap::new();
    let mut missing = 0u64;

    for block_num in start..=end {
        let Some(header) = store.get_block_header(block_num)? else {
            missing += 1;
            continue;
        };
        let block_hash = header.hash();

        let Some(body) = store.get_block_body_by_hash(block_hash).await? else {
            missing += 1;
            continue;
        };

        let receipts = store
            .get_receipts_for_block(block_hash)
            .await?
            .unwrap_or_default();

        let total_txs = body.transactions.len();

        // Pair transactions with their receipts. If receipts are missing or
        // mismatched (should not happen on a healthy DB) we conservatively
        // treat those txs as failed.
        let create_txs = body
            .transactions
            .iter()
            .filter(|tx| tx.is_contract_creation())
            .count();

        let successful_creates = body
            .transactions
            .iter()
            .zip(receipts.iter())
            .filter(|(tx, receipt)| tx.is_contract_creation() && receipt.succeeded)
            .count();

        let gas_used = receipts.last().map(|r| r.cumulative_gas_used).unwrap_or(0);

        for tx in &body.transactions {
            if let TxKind::Call(to) = tx.to() {
                *call_counts.entry(to).or_insert(0) += 1;
            }
        }

        if let Some(ref mut f) = csv_writer {
            writeln!(
                f,
                "{block_num},{total_txs},{create_txs},{successful_creates},{gas_used}"
            )?;
        }

        stats.push(BlockStats {
            block_number: block_num,
            total_txs,
            create_txs,
            successful_creates,
            gas_used,
        });

        if block_num % 1_000 == 0 {
            eprintln!("  ... processed block {block_num}");
        }
    }

    println!(
        "\n=== Results over {} blocks ({missing} skipped) ===\n",
        stats.len()
    );

    let total_creates: u64 = stats.iter().map(|s| s.successful_creates as u64).sum();
    let total_txs: u64 = stats.iter().map(|s| s.total_txs as u64).sum();
    println!("Total transactions      : {total_txs}");
    println!("Total successful creates: {total_creates}");
    if total_txs > 0 {
        println!(
            "Deploy rate             : {:.3}%",
            total_creates as f64 / total_txs as f64 * 100.0
        );
    }
    println!();

    print_summary(
        "creates/block (attempted) ",
        stats.iter().map(|s| s.create_txs as u64).collect(),
    );
    print_summary(
        "creates/block (successful)",
        stats.iter().map(|s| s.successful_creates as u64).collect(),
    );
    print_summary(
        "txs/block                 ",
        stats.iter().map(|s| s.total_txs as u64).collect(),
    );
    print_summary(
        "gas_used/block            ",
        stats.iter().map(|s| s.gas_used).collect(),
    );

    // Top 10 blocks by deployment count.
    let mut by_creates: Vec<&BlockStats> = stats.iter().collect();
    by_creates.sort_unstable_by_key(|s| std::cmp::Reverse(s.successful_creates));
    println!("\nTop 10 blocks by successful contract deployments:");
    println!(
        "{:>12}  {:>9}  {:>8}  {:>10}",
        "block", "creates", "total_tx", "gas_used"
    );
    for s in by_creates.iter().take(10) {
        println!(
            "{:>12}  {:>9}  {:>8}  {:>10}",
            s.block_number, s.successful_creates, s.total_txs, s.gas_used
        );
    }

    // Top-N most-called contracts.
    let total_calls: u64 = call_counts.values().sum();
    let mut by_calls: Vec<(Address, u64)> = call_counts.into_iter().collect();
    by_calls.sort_unstable_by_key(|(_, count)| std::cmp::Reverse(*count));

    println!(
        "\nTop {} most-called contracts ({} unique addresses, {} total calls):",
        cli.top_n,
        by_calls.len(),
        total_calls
    );
    println!(
        "{:>5}  {:<42}  {:>10}  {:>8}",
        "rank", "address", "calls", "share%"
    );
    for (rank, (addr, count)) in by_calls.iter().take(cli.top_n).enumerate() {
        println!(
            "{:>5}  {addr:#x}  {:>10}  {:>7.3}%",
            rank + 1,
            count,
            *count as f64 / total_calls as f64 * 100.0,
        );
    }

    Ok(())
}
