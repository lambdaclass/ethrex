use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

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

    // Try to find a replayable starting point.
    // The trie layer cache may not be persisted, so we scan forward
    // from `from` until we find a block whose parent state root is available.
    let actual_from = find_first_replayable_block(&store, from, to).await?;
    if actual_from > from {
        info!(
            "State root unavailable for blocks {}..{}, starting replay at block {}",
            from,
            actual_from - 1,
            actual_from
        );
    }

    let actual_count = to.saturating_sub(actual_from) + 1;
    let skipped = actual_from.saturating_sub(from);

    let mut all_timings: Vec<BlockTimings> = Vec::with_capacity(actual_count as usize);
    let mut errors = 0u64;
    let wall_start = Instant::now();

    for number in actual_from..=to {
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
                info!(
                    "Block {} replayed: pipeline={}ms exec={}ms merkle={}ms store={}ms gas={}",
                    number,
                    timings.pipeline_total.as_millis(),
                    timings.executor.as_millis(),
                    timings.merkle_drain.as_millis(),
                    timings.store.as_millis(),
                    timings.gas_used,
                );
                all_timings.push(timings);
            }
            Err(e) => {
                info!("Block {} replay failed: {}", number, e);
                errors += 1;
            }
        }
    }

    let wall_elapsed = wall_start.elapsed();

    if all_timings.is_empty() {
        println!("No blocks successfully replayed.");
        println!("Hint: the trie layer cache may not be persisted. Try replaying while");
        println!("the node has recently processed blocks, or from an earlier block range.");
        return Ok(());
    }

    print_statistics(&all_timings, actual_from, to, errors, skipped, wall_elapsed);

    if let Some(path) = csv_path {
        write_csv(&all_timings, &path)?;
        println!("\nCSV written to: {}", path.display());
    }

    Ok(())
}

/// Scan forward from `from` to `to` to find the first block whose parent
/// state root is available in the store (i.e., the trie can be opened).
async fn find_first_replayable_block(store: &Store, from: u64, to: u64) -> eyre::Result<u64> {
    for number in from..=to {
        let Some(block_hash) = store.get_canonical_block_hash(number).await? else {
            continue;
        };
        // We need the PARENT's state root, so get the parent header
        let Some(header) = store.get_block_header_by_hash(block_hash)? else {
            continue;
        };
        let Some(parent_header) = store.get_block_header_by_hash(header.parent_hash)? else {
            continue;
        };
        if store.has_state_root(parent_header.state_root)? {
            return Ok(number);
        }
    }
    // No replayable block found; return from and let the loop handle errors
    Ok(from)
}

fn print_statistics(
    timings: &[BlockTimings],
    from: u64,
    to: u64,
    errors: u64,
    skipped: u64,
    wall_elapsed: Duration,
) {
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
        "\n=== Replay Results: blocks {}..{} ({} replayed, {} errors, {} skipped) ===",
        from, to, n, errors, skipped
    );
    println!(
        "Total gas: {} | Pipeline time: {}ms | Wall time: {:.1}s | Throughput: {:.3} Ggas/s\n",
        total_gas,
        total_pipeline_ms,
        wall_elapsed.as_secs_f64(),
        avg_ggas_per_s,
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
