use std::path::PathBuf;
use std::time::Duration;

use ethrex_blockchain::Blockchain;
use ethrex_blockchain::timings::BlockTimings;
use ethrex_storage::Store;
use tracing::info;

pub async fn run_replay(
    store: Store,
    blockchain: &Blockchain,
    from: u64,
    to: u64,
    csv_path: Option<PathBuf>,
) -> eyre::Result<()> {
    let block_count = to.saturating_sub(from) + 1;
    info!("Replaying blocks {}..{} ({} blocks)", from, to, block_count);

    let mut all_timings: Vec<BlockTimings> = Vec::with_capacity(block_count as usize);
    let mut errors = 0u64;

    for number in from..=to {
        let block_hash = match store.get_canonical_block_hash(number).await? {
            Some(hash) => hash,
            None => {
                info!("Block {} not found in store, skipping", number);
                errors += 1;
                continue;
            }
        };

        let block = match store.get_block_by_hash(block_hash).await? {
            Some(block) => block,
            None => {
                info!("Block body {} not found, skipping", number);
                errors += 1;
                continue;
            }
        };

        match blockchain.replay_block(&block) {
            Ok(timings) => {
                all_timings.push(timings);
            }
            Err(e) => {
                info!("Block {} replay failed: {}", number, e);
                errors += 1;
            }
        }
    }

    if all_timings.is_empty() {
        println!("No blocks successfully replayed.");
        return Ok(());
    }

    print_statistics(&all_timings, from, to, errors);

    if let Some(path) = csv_path {
        write_csv(&all_timings, &path)?;
        println!("\nCSV written to: {}", path.display());
    }

    Ok(())
}

fn print_statistics(timings: &[BlockTimings], from: u64, to: u64, errors: u64) {
    let n = timings.len();

    let total_gas: u64 = timings.iter().map(|t| t.gas_used).sum();
    let total_pipeline: Duration = timings.iter().map(|t| t.pipeline_total).sum();
    let total_pipeline_ms = total_pipeline.as_millis();
    let avg_ggas_per_s = if total_pipeline_ms > 0 {
        (total_gas as f64 / 1e9) / (total_pipeline_ms as f64 / 1000.0)
    } else {
        0.0
    };

    println!(
        "\n=== Replay Results: blocks {}..{} ({} replayed, {} errors) ===",
        from, to, n, errors
    );
    println!(
        "Total gas: {} | Total pipeline time: {}ms | Avg throughput: {:.3} Ggas/s\n",
        total_gas, total_pipeline_ms, avg_ggas_per_s,
    );

    println!(
        "{:<20} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "Phase", "Avg(ms)", "Med(ms)", "P95(ms)", "P99(ms)", "Max(ms)"
    );
    println!("{}", "-".repeat(72));

    let phases: Vec<(&str, Vec<u128>)> = vec![
        (
            "pipeline",
            timings.iter().map(|t| t.pipeline_total.as_millis()).collect(),
        ),
        (
            "  exec",
            timings.iter().map(|t| t.executor.as_millis()).collect(),
        ),
        (
            "  merkle_drain",
            timings.iter().map(|t| t.merkle_drain.as_millis()).collect(),
        ),
        (
            "  store",
            timings.iter().map(|t| t.store.as_millis()).collect(),
        ),
        (
            "    store_write",
            timings.iter().map(|t| t.store_db_write.as_millis()).collect(),
        ),
        (
            "    store_trie",
            timings.iter().map(|t| t.store_trie_wait.as_millis()).collect(),
        ),
        (
            "    store_commit",
            timings.iter().map(|t| t.store_db_commit.as_millis()).collect(),
        ),
        (
            "  validate",
            timings.iter().map(|t| t.validate.as_millis()).collect(),
        ),
        (
            "  warmer",
            timings.iter().map(|t| t.warmer.as_millis()).collect(),
        ),
    ];

    for (name, mut values) in phases {
        values.sort();
        let avg = values.iter().sum::<u128>() as f64 / n as f64;
        let med = values[n / 2];
        let p95 = values[((n - 1) as f64 * 0.95) as usize];
        let p99 = values[((n - 1) as f64 * 0.99) as usize];
        let max = values[n - 1];

        println!(
            "{:<20} {:>8.1} {:>8} {:>8} {:>8} {:>8}",
            name, avg, med, p95, p99, max
        );
    }
}

fn write_csv(timings: &[BlockTimings], path: &PathBuf) -> eyre::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    writeln!(
        f,
        "block,gas,txs,pipeline_ms,exec_ms,merkle_drain_ms,store_ms,store_write_ms,store_trie_ms,store_commit_ms,validate_ms,warmer_ms"
    )?;
    for t in timings {
        writeln!(
            f,
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            t.block_number,
            t.gas_used,
            t.tx_count,
            t.pipeline_total.as_millis(),
            t.executor.as_millis(),
            t.merkle_drain.as_millis(),
            t.store.as_millis(),
            t.store_db_write.as_millis(),
            t.store_trie_wait.as_millis(),
            t.store_db_commit.as_millis(),
            t.validate.as_millis(),
            t.warmer.as_millis(),
        )?;
    }
    Ok(())
}
