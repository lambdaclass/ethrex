//! Re-derives the transaction and receipt roots of every stored block from the
//! bodies and receipts on disk, and compares them to the (consensus-validated)
//! headers.
//!
//! This is the offline, full-coverage counterpart to spot-checking against a
//! block explorer. It needs no network and no third party, and unlike a
//! fetch-time check it also catches corruption that happened *after* the write
//! (bit rot, a bad compaction), because it recomputes from what is actually on
//! disk right now.
//!
//! RocksDB is single-process, so point this at a snapshot (a hard-link copy of a
//! cleanly-stopped datadir) rather than at a live node's directory.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

use clap::Parser;
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{compute_receipts_root, validate_block_body};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

#[derive(Parser)]
#[command(about = "Re-derive tx/receipt roots for a block range and compare to headers")]
struct Args {
    /// Datadir to validate (use a snapshot, not a live node's directory).
    #[arg(long)]
    datadir: String,
    #[arg(long)]
    first: u64,
    #[arg(long)]
    last: u64,
    /// Parallel workers. Each opens its own read path over the same store.
    #[arg(long, default_value_t = 12)]
    workers: u64,
}

#[derive(Default)]
struct Counts {
    checked: AtomicU64,
    missing_body: AtomicU64,
    missing_header: AtomicU64,
    bad_body: AtomicU64,
    bad_receipts: AtomicU64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let store = Store::new(&args.datadir, EngineType::RocksDB).expect("open store");
    let counts = Arc::new(Counts::default());
    let total = args.last - args.first + 1;
    let started = Instant::now();

    // Split the range into contiguous shards, one task each. Contiguity keeps the
    // reads sequential per shard, which matters far more than perfect balance.
    let per = total.div_ceil(args.workers);
    let mut tasks = Vec::new();
    for w in 0..args.workers {
        let lo = args.first + w * per;
        if lo > args.last {
            break;
        }
        let hi = (lo + per - 1).min(args.last);
        let store = store.clone();
        let counts = counts.clone();
        tasks.push(tokio::spawn(async move {
            let crypto = NativeCrypto;
            for n in lo..=hi {
                let Ok(Some(header)) = store.get_block_header(n) else {
                    counts.missing_header.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let Ok(Some(body)) = store.get_block_body(n).await else {
                    counts.missing_body.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                // `validate_block_body` covers transactions_root and
                // withdrawals_root. It does *not* look at ommers_hash: it only
                // asserts the ommers list is empty (EIP-3675). That is correct for
                // post-merge blocks but leaves the header field unchecked, so tie
                // the two together here. Note this makes the tool post-merge only,
                // which matches the range backfill can reach (>= Byzantium is the
                // floor, and pre-merge blocks would need real ommers hashing).
                if body.ommers.is_empty() && header.ommers_hash != *DEFAULT_OMMERS_HASH {
                    counts.bad_body.fetch_add(1, Ordering::Relaxed);
                    println!(
                        "BAD OMMERS block {n} (empty body ommers, header says {:?})",
                        header.ommers_hash
                    );
                }
                if validate_block_body(&header, &body, &crypto).is_err() {
                    counts.bad_body.fetch_add(1, Ordering::Relaxed);
                    println!("BAD BODY   block {n}");
                }
                let receipts = store
                    .get_receipts_for_block(&header.hash())
                    .await
                    .unwrap_or_default();
                if compute_receipts_root(&receipts, &crypto) != header.receipts_root {
                    counts.bad_receipts.fetch_add(1, Ordering::Relaxed);
                    println!(
                        "BAD RECEIPTS block {n} (stored {} receipts, {} txs)",
                        receipts.len(),
                        body.transactions.len()
                    );
                }
                let done = counts.checked.fetch_add(1, Ordering::Relaxed) + 1;
                if done % 250_000 == 0 {
                    let rate = done as f64 / started.elapsed().as_secs_f64();
                    println!(
                        "  {done}/{total} checked  {rate:.0} blk/s  eta {:.0} min",
                        (total - done) as f64 / rate / 60.0
                    );
                }
            }
        }));
    }
    for t in tasks {
        let _ = t.await;
    }

    let secs = started.elapsed().as_secs_f64();
    println!("\n===== RESULT =====");
    println!("range              {}..={}", args.first, args.last);
    println!(
        "blocks checked     {}",
        counts.checked.load(Ordering::Relaxed)
    );
    println!(
        "missing header     {}",
        counts.missing_header.load(Ordering::Relaxed)
    );
    println!(
        "missing body       {}",
        counts.missing_body.load(Ordering::Relaxed)
    );
    println!(
        "BAD BODY ROOTS     {}",
        counts.bad_body.load(Ordering::Relaxed)
    );
    println!(
        "BAD RECEIPT ROOTS  {}",
        counts.bad_receipts.load(Ordering::Relaxed)
    );
    println!("elapsed            {:.0}s ({:.1} min)", secs, secs / 60.0);
}
