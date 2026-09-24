//! Times the last block of each blockchain fixture and reports MGas/s.
//!
//! Built for EEST gas benchmarks (`fill --gas-benchmark-values N`): every block but
//! the last is setup, and the last one is the workload. Two measurements per fixture:
//!
//! - `exec`: sequential LEVM execution of the block (`Evm::execute_block`) on one
//!   thread, without merkleization or storage.
//! - `pipeline`: `add_block_pipeline` with the block's BAL, the `newPayload` path:
//!   BAL-parallel execution, merkleization and storage. It uses several cores, and
//!   `--exec-only` skips it.
//!
//! Prints one tab-separated row per fixture, so a whole benchmark suite can be
//! ranked by throughput.
//!
//! Usage: `block_bench <fixture file or dir> [--runs N] [--filter SUBSTRING] [--exec-only]`

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use ef_tests_blockchain::{
    test_runner::{build_store_for_test, parse_tests},
    types::TestUnit,
};
use ethrex_blockchain::{Blockchain, fork_choice::apply_fork_choice, vm::StoreVmDatabase};
use ethrex_common::types::{Block, block_access_list::BlockAccessList};
use ethrex_storage::Store;

const USAGE: &str =
    "usage: block_bench <fixture file or dir> [--runs N] [--filter S] [--exec-only]";

struct Args {
    path: PathBuf,
    runs: usize,
    filter: Option<String>,
    exec_only: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut runs = 10;
    let mut filter = None;
    let mut exec_only = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runs" => {
                runs = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or("--runs needs a positive integer")?;
            }
            "--filter" => filter = Some(args.next().ok_or("--filter needs a value")?),
            "--exec-only" => exec_only = true,
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => return Err(format!("unexpected argument: {arg}\n{USAGE}")),
        }
    }
    let path = path.ok_or(USAGE)?;
    Ok(Args {
        path,
        runs,
        filter,
        exec_only,
    })
}

/// Every `.json` file under `path`, sorted, so a suite is parsed one file at a time
/// rather than all at once.
fn fixture_files(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files = Vec::new();
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("cannot read {}", dir.display());
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Builds the genesis state and imports every block but the last, which it returns.
async fn setup_chain(
    test: &TestUnit,
    pool: &Arc<rayon::ThreadPool>,
) -> Result<(Store, Blockchain, Block), String> {
    let store = build_store_for_test(test).await;
    let blockchain = Blockchain::for_test_harness_with_pool(store.clone(), Arc::clone(pool));
    let mut blocks = test.blocks.iter().map(|fixture| {
        fixture
            .block()
            .cloned()
            .map(Block::from)
            .ok_or("block fixture without a block")
    });
    let mut last = blocks.next().ok_or("fixture has no blocks")??;
    for next in blocks {
        let hash = last.hash();
        blockchain
            .add_block_pipeline(last, None)
            .map_err(|e| format!("setup block failed: {e:?}"))?;
        apply_fork_choice(&store, hash, hash, hash, None)
            .await
            .map_err(|e| format!("setup fork choice failed: {e:?}"))?;
        last = next?;
    }
    Ok((store, blockchain, last))
}

/// Executes `block` sequentially on a fresh VM over its parent state.
fn time_exec(
    store: &Store,
    blockchain: &Blockchain,
    block: &Block,
) -> Result<(Duration, Option<BlockAccessList>), String> {
    let parent = store
        .get_block_header_by_hash(block.header.parent_hash)
        .map_err(|e| format!("{e:?}"))?
        .ok_or("parent header not found")?;
    let vm_db = StoreVmDatabase::new(store.clone(), parent).map_err(|e| format!("{e:?}"))?;
    let mut vm = blockchain.new_evm(vm_db).map_err(|e| format!("{e:?}"))?;

    let start = Instant::now();
    let (result, bal) = vm.execute_block(block).map_err(|e| format!("{e:?}"))?;
    let elapsed = start.elapsed();

    if result.block_gas_used != block.header.gas_used {
        return Err(format!(
            "gas used mismatch: executed {}, header {}",
            result.block_gas_used, block.header.gas_used
        ));
    }
    Ok((elapsed, bal))
}

/// Imports `block` through the full pipeline onto a freshly built chain.
async fn time_pipeline(
    test: &TestUnit,
    pool: &Arc<rayon::ThreadPool>,
    bal: Option<Arc<BlockAccessList>>,
) -> Result<Duration, String> {
    let (_store, blockchain, block) = setup_chain(test, pool).await?;
    let start = Instant::now();
    blockchain
        .add_block_pipeline(block, bal)
        .map_err(|e| format!("{e:?}"))?;
    Ok(start.elapsed())
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn mgas_per_s(gas: u64, time: Duration) -> f64 {
    gas as f64 / time.as_secs_f64() / 1e6
}

/// `test_fn[params]` from a pytest id, without the fork, format and gas-value
/// parameters every benchmark fixture shares.
fn short_name(key: &str) -> String {
    let name = key.rsplit_once("::").map_or(key, |(_, name)| name);
    let Some((func, params)) = name
        .split_once('[')
        .and_then(|(func, rest)| Some((func, rest.strip_suffix(']')?)))
    else {
        return name.to_string();
    };
    let shared = |p: &str| {
        p.starts_with("fork_")
            || p.starts_with("blockchain_test")
            || matches!(p, "benchmark" | "gas")
            || (p.starts_with("value_") && p.ends_with('M'))
    };
    let params: Vec<_> = params.split('-').filter(|p| !shared(p)).collect();
    format!("{func}[{}]", params.join("-"))
}

struct Sample {
    gas: u64,
    txs: usize,
    exec: Duration,
    pipeline: Option<Duration>,
}

async fn bench(
    test: &TestUnit,
    runs: usize,
    exec_only: bool,
    pool: &Arc<rayon::ThreadPool>,
) -> Result<Sample, String> {
    let (store, blockchain, block) = setup_chain(test, pool).await?;

    // One untimed run warms the store caches and yields the BAL.
    let (_, bal) = time_exec(&store, &blockchain, &block)?;
    let exec = (0..runs)
        .map(|_| time_exec(&store, &blockchain, &block).map(|(time, _)| time))
        .collect::<Result<Vec<_>, _>>()?;

    let pipeline = if exec_only {
        None
    } else {
        let bal = bal.map(Arc::new);
        time_pipeline(test, pool, bal.clone()).await?;
        let mut samples = Vec::with_capacity(runs);
        for _ in 0..runs {
            samples.push(time_pipeline(test, pool, bal.clone()).await?);
        }
        Some(median(samples))
    };

    Ok(Sample {
        gas: block.header.gas_used,
        txs: block.body.transactions.len(),
        exec: median(exec),
        pipeline,
    })
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args = parse_args()?;
    let pool = Blockchain::build_merkle_pool();

    println!("mgas\ttxs\texec_ms\texec_mgas_s\tpipe_ms\tpipe_mgas_s\ttest");
    for file in fixture_files(&args.path) {
        let mut tests: Vec<_> = parse_tests(&file)
            .into_iter()
            .filter(|(key, _)| args.filter.as_ref().is_none_or(|f| key.contains(f)))
            .collect();
        tests.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (key, test) in &tests {
            if test.blocks.iter().any(|b| b.expect_exception.is_some()) {
                eprintln!("skipping {key}: expects an exception");
                continue;
            }
            match bench(test, args.runs, args.exec_only, &pool).await {
                Ok(s) => {
                    let (pipe_ms, pipe_mgas) = s.pipeline.map_or((f64::NAN, f64::NAN), |p| {
                        (p.as_secs_f64() * 1e3, mgas_per_s(s.gas, p))
                    });
                    println!(
                        "{:.1}\t{}\t{:.2}\t{:.1}\t{:.2}\t{:.1}\t{}",
                        s.gas as f64 / 1e6,
                        s.txs,
                        s.exec.as_secs_f64() * 1e3,
                        mgas_per_s(s.gas, s.exec),
                        pipe_ms,
                        pipe_mgas,
                        short_name(key),
                    );
                }
                Err(e) => eprintln!("{key}: {e}"),
            }
        }
    }
    Ok(())
}
