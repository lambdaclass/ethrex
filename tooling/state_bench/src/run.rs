//! `run`: the timed cold-state import benchmark + its subprocess workers.
//!
//! # Why subprocesses
//!
//! Phase 2 established that an in-process `drop(store)` does NOT release the
//! RocksDB `LOCK`: the `Store` spawns persist + flat-KV background threads that
//! retain `Arc` clones of the backend, so the same datadir cannot be reopened
//! within one process, and the block cache would carry over warm. Therefore each
//! step of a run is a *fresh subprocess* — a re-exec of this same binary
//! (`std::env::current_exe()`) invoked with a HIDDEN internal subcommand. Process
//! exit unconditionally releases the RocksDB lock AND gives a genuinely cold
//! block cache (fresh address space). The parent `run` orchestrates:
//!
//! ```text
//! _warmup (once)  -> record undo log to a file + save the pristine state digest
//! for i in 1..=N {
//!     [optional drop_caches]
//!     _measure i  -> cold import, emit one metrics line, coldness self-check
//!     _reset      -> replay undo log, assert state byte-equals pristine
//! }
//! ```
//!
//! The metrics line written by `_measure` is intentionally stable and greppable
//! (`key=value` tokens) because Phase 6 (`compare`) parses it.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use tracing::{info, warn};

use ethrex_blockchain::Blockchain;
use ethrex_common::types::Block;
use ethrex_common::types::Genesis;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::api::StorageBackend;
use ethrex_storage::backend::rocksdb::{RocksDBBackend, RocksDbOpenOpts};
use ethrex_storage::{DB_COMMIT_THRESHOLD, Store, StoreConfig};

use state_bench::recording_backend::{
    RecordingBackend, apply_undo_log, load_undo_log, save_undo_log, state_digest,
};

/// Default RocksDB block cache size for a cold run: 64 MiB. Small on purpose so
/// the cache cannot hold the working set — most reads must hit disk.
const DEFAULT_BLOCK_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Default number of measured runs when `--runs` is omitted.
const DEFAULT_RUNS: usize = 3;

/// Minimum merkleization-pool size. The parallel BAL merkle path spawns 16
/// cross-communicating worker tasks plus a watcher on this pool, all of which
/// must be live at once (each worker blocks until it receives `RoutingDone` from
/// all 16), so a smaller pool deadlocks. Matches `Blockchain::build_merkle_pool`'s
/// hardcoded 17-thread default. `--jobs` sizes the pool upward from this floor.
const MERKLE_POOL_MIN_THREADS: usize = 17;

// --- Coldness self-check floors ------------------------------------------------
//
// After a measured run we assert the RocksDB stats *deltas* clear these floors.
// A silently-warm datadir (direct reads not applied, block cache not fresh, OS
// page cache retained) would serve reads from cache and produce near-zero disk
// activity, so these floors catch broken cold plumbing and fail the run loud.
//
// `block_cache_miss` and `sst_read_count` are the strong signals: with a fresh
// 64 MiB cache and O_DIRECT, cold reads MUST miss the cache and MUST issue SST
// file reads, so a floor of >0 on each is decisive. `bytes_read` counts bytes
// served to `Get()` from any source (memtable/cache/SST), so it stays non-zero
// even when warm; its floor is therefore only a low, workload-size-independent
// sanity bound (did the import read a non-trivial amount at all), NOT a coldness
// signal — a completely broken run reads ~nothing, but even a small smoke
// workload of a few hundred touches reads hundreds of KB. Coldness itself is
// enforced by the two miss/SST floors above.
const MIN_CACHE_MISS: u64 = 1;
const MIN_BYTES_READ: u64 = 64 * 1024; // 64 KiB sanity floor
const MIN_SST_READS: u64 = 1;

/// Which reset strategy restores the datadir to pristine between measured runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ResetMode {
    /// Replay the warmup's undo log directly onto the datadir's column families
    /// and assert the post-undo state digest byte-equals the saved pristine
    /// digest. Fast and needs no extra disk, but mutates the datadir in place, so
    /// async compaction can rewrite SSTs differently between cycles and add
    /// cache-miss variance; a failed measure also leaves the datadir dirty.
    Undo,
    /// Snapshot the pristine datadir once (RocksDB checkpoint), then run each
    /// measured import on a fresh per-run copy of that snapshot, deleting the
    /// copy afterward. Slower and disk-hungry (a full datadir copy per run) but
    /// never mutates the original datadir and hardlinks the same immutable
    /// pristine SSTs each run, giving a stable physical layout and reproducible
    /// cold metrics. Default mode.
    Checkpoint,
}

/// Column families / options shared by every subprocess that opens the datadir.
#[derive(Args, Debug, Clone)]
pub struct ColdDbArgs {
    /// Datadir to open (the pristine gen-state fixture for undo mode, or a
    /// per-run copy for checkpoint mode).
    #[arg(long)]
    pub datadir: PathBuf,
    /// RocksDB block cache size in bytes.
    #[arg(long, default_value_t = DEFAULT_BLOCK_CACHE_BYTES)]
    pub block_cache_bytes: usize,
    /// Open RocksDB with `O_DIRECT` reads (bypass the OS page cache). Takes an
    /// explicit value so the parent can forward its own setting verbatim.
    #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
    pub direct_reads: bool,
}

/// Parameters parsed from the parent `run` subcommand.
#[derive(Args, Debug)]
pub struct RunArgs {
    /// Pristine datadir produced by `gen-state` (holds `metadata.json`).
    #[arg(long)]
    pub datadir: PathBuf,
    /// RLP-concatenated workload blocks (`chain.rlp` from `gen-workload`).
    #[arg(long)]
    pub chain: PathBuf,
    /// RLP-concatenated BALs (`bals.rlp` from `gen-workload`).
    #[arg(long)]
    pub bals: PathBuf,
    /// Base genesis (Amsterdam-activated, e.g. `fixtures/genesis/l1-bal.json`).
    /// Its chain config is re-applied on every reopen (the store does not
    /// persist chain config).
    #[arg(long)]
    pub genesis: PathBuf,
    /// Number of measured runs.
    #[arg(long, default_value_t = DEFAULT_RUNS)]
    pub runs: usize,
    /// Reset strategy between measured runs. Defaults to `checkpoint` for
    /// reproducibility: it hardlinks the immutable pristine SSTs into a fresh
    /// per-run copy, giving a stable physical layout each run. `undo` mutates the
    /// datadir in place and its async compaction can rewrite SSTs differently
    /// each cycle, adding cache-miss variance to the cold metrics.
    #[arg(long, value_enum, default_value_t = ResetMode::Checkpoint)]
    pub reset: ResetMode,
    /// Attempt `echo 3 > /proc/sys/vm/drop_caches` before each measured run to
    /// evict the OS page cache. Warns and continues if it lacks privilege.
    #[arg(long, default_value_t = false)]
    pub drop_caches: bool,
    /// Open RocksDB with `O_DIRECT` reads (bypass the OS page cache).
    #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
    pub direct_reads: bool,
    /// RocksDB block cache size in bytes.
    #[arg(long, default_value_t = DEFAULT_BLOCK_CACHE_BYTES)]
    pub block_cache_bytes: usize,
    /// Append one metrics line per measured run to this path (truncated at the
    /// start of the run).
    #[arg(long)]
    pub out_log: PathBuf,
}

/// Hidden `_warmup` worker args.
#[derive(Args, Debug)]
pub struct WarmupArgs {
    #[command(flatten)]
    pub db: ColdDbArgs,
    #[arg(long)]
    pub chain: PathBuf,
    #[arg(long)]
    pub bals: PathBuf,
    #[arg(long)]
    pub genesis: PathBuf,
    /// Where to write the captured undo log.
    #[arg(long)]
    pub undo_log: PathBuf,
    /// Where to write the pristine state digest (hex).
    #[arg(long)]
    pub pristine_digest: PathBuf,
}

/// Hidden `_measure` worker args.
#[derive(Args, Debug)]
pub struct MeasureArgs {
    #[command(flatten)]
    pub db: ColdDbArgs,
    #[arg(long)]
    pub chain: PathBuf,
    #[arg(long)]
    pub bals: PathBuf,
    #[arg(long)]
    pub genesis: PathBuf,
    /// 1-based index of this measured run (recorded in the metrics line).
    #[arg(long)]
    pub run_index: usize,
    /// Metrics line is appended here.
    #[arg(long)]
    pub out_log: PathBuf,
}

/// Hidden `_reset` worker args.
#[derive(Args, Debug)]
pub struct ResetArgs {
    #[command(flatten)]
    pub db: ColdDbArgs,
    /// Undo log written by `_warmup`.
    #[arg(long)]
    pub undo_log: PathBuf,
    /// Pristine digest written by `_warmup`; the post-undo digest must equal it.
    #[arg(long)]
    pub pristine_digest: PathBuf,
}

// =============================================================================
// Shared open + import helpers (used by _warmup and _measure)
// =============================================================================

/// Decode the workload: blocks from `chain.rlp`, BALs from `bals.rlp`. Framing
/// mirrors `gen-workload`'s writers and the `--with-bal` decoder at cli.rs.
fn load_workload(chain: &Path, bals: &Path) -> Result<(Vec<Block>, Vec<Arc<BlockAccessList>>)> {
    let chain_bytes =
        std::fs::read(chain).with_context(|| format!("reading chain file {}", chain.display()))?;
    let mut rest = chain_bytes.as_slice();
    let mut blocks = Vec::new();
    while !rest.is_empty() {
        let (block, tail) =
            Block::decode_unfinished(rest).context("decoding block from chain file")?;
        blocks.push(block);
        rest = tail;
    }

    let bal_bytes =
        std::fs::read(bals).with_context(|| format!("reading BAL file {}", bals.display()))?;
    let mut rest = bal_bytes.as_slice();
    let mut decoded_bals = Vec::new();
    while !rest.is_empty() {
        let (bal, tail) =
            BlockAccessList::decode_unfinished(rest).context("decoding BAL from BAL file")?;
        decoded_bals.push(Arc::new(bal));
        rest = tail;
    }

    // Mirror cli.rs:1050: one BAL per Amsterdam+ block, else the parallel import
    // path would silently fall through to sequential execution.
    let amsterdam_blocks = blocks
        .iter()
        .filter(|b| b.header.block_access_list_hash.is_some())
        .count();
    if decoded_bals.len() != amsterdam_blocks {
        bail!(
            "BAL file has {} entries but chain has {} Amsterdam+ blocks; mismatched BAL files \
             would fall through to sequential execution and produce misleading numbers",
            decoded_bals.len(),
            amsterdam_blocks
        );
    }

    Ok((blocks, decoded_bals))
}

/// Parse the base genesis JSON (its chain config is re-applied on reopen).
fn load_genesis(genesis: &Path) -> Result<Genesis> {
    let bytes = std::fs::read(genesis)
        .with_context(|| format!("reading base genesis {}", genesis.display()))?;
    serde_json::from_slice(&bytes).context("parsing base genesis JSON")
}

/// Build a `Store` over an already-opened backend and re-apply the chain config
/// from `genesis` (the store does not persist chain config across reopen, so
/// every subprocess that opens the datadir must do this, exactly as
/// `gen-workload` does).
async fn build_store(
    backend: Arc<dyn StorageBackend>,
    datadir: &Path,
    genesis: &Genesis,
) -> Result<Store> {
    let persist_capacity = StoreConfig::default().persist_channel_capacity;
    let mut store = Store::from_backend_bench(
        backend,
        datadir.to_path_buf(),
        DB_COMMIT_THRESHOLD,
        persist_capacity,
    )
    .context("building Store over the cold backend (requires metadata.json)")?;
    store
        .set_chain_config(&genesis.config)
        .await
        .context("re-applying chain config on reopen")?;
    store
        .load_initial_state()
        .await
        .context("anchoring the store head to the durable genesis")?;
    Ok(store)
}

/// Import the workload block-by-block through the parallel BAL path, mirroring
/// `import_blocks_bench`'s `--with-bal` loop: per block `add_block_pipeline`
/// with its preloaded BAL, then `wait_for_persistence_idle` so the next block's
/// timing does not absorb this block's background persistence.
async fn import_loop(
    blockchain: &Blockchain,
    store: &Store,
    blocks: &[Block],
    bals: &[Arc<BlockAccessList>],
) -> Result<()> {
    let mut bal_index = 0usize;
    for block in blocks {
        let number = block.header.number;
        // BALs exist only for Amsterdam+ blocks; advance a separate cursor for
        // exactly those, matching cli.rs:1109.
        let bal = if block.header.block_access_list_hash.is_some() {
            let b = bals.get(bal_index).cloned();
            bal_index += 1;
            b
        } else {
            None
        };
        blockchain
            .add_block_pipeline(block.clone(), bal)
            .with_context(|| format!("add_block_pipeline for block {number}"))?;
        store
            .wait_for_persistence_idle()
            .await
            .with_context(|| format!("wait_for_persistence_idle after block {number}"))?;
    }
    Ok(())
}

/// Make the imported head canonical (single forkchoice update, like cli.rs),
/// then drain the persist worker.
///
/// Cold WRITES are flushed by the store's *rolling* commit: the persist worker
/// commits the bottom trie diff-layer to disk as soon as the in-memory layer
/// chain reaches `commit_threshold` (128) deep, which happens continuously
/// during the per-block import loop (see `commit_trie_if_due`). So the bulk of
/// cold-write I/O lands inside `loop_seconds`, and this finalize step only
/// covers the canonical labeling plus draining any final in-flight flush. The
/// most recent ~128 blocks' layers remain in memory and are not flushed (they
/// are the unmeasured tail; use a large workload so the tail is a small
/// fraction). `_warmup` and `_measure` run this identical sequence, so the undo
/// log captures pre-images for exactly the writes `_measure` performs.
async fn finalize_head(store: &Store, blocks: &[Block]) -> Result<()> {
    let mut canonical: Vec<_> = blocks.iter().map(|b| (b.header.number, b.hash())).collect();
    let Some((head_number, head_hash)) = canonical.pop() else {
        bail!("workload has no blocks");
    };
    store
        .forkchoice_update(
            canonical,
            head_number,
            head_hash,
            Some(head_number),
            Some(head_number),
        )
        .await
        .context("final forkchoice_update")?;
    store
        .wait_for_persistence_idle()
        .await
        .context("final wait_for_persistence_idle (drains any in-flight flush)")?;
    Ok(())
}

/// Build the parallel-import `Blockchain` over `store`, sizing the merkleization
/// thread pool from `jobs` so `--jobs` controls real import parallelism instead
/// of only labelling the metrics line.
///
/// The pool is floored at [`MERKLE_POOL_MIN_THREADS`]: the parallel BAL merkle
/// path spawns 16 cross-communicating worker tasks plus a watcher that must all
/// run concurrently (each worker blocks until it has received `RoutingDone` from
/// all 16), so a pool smaller than that deadlocks. `--jobs` therefore scales the
/// pool upward from the floor; below the floor it is clamped (a warning is
/// logged) to keep the import correct.
fn build_blockchain(store: Store, jobs: usize) -> Blockchain {
    let threads = jobs.max(MERKLE_POOL_MIN_THREADS);
    if threads != jobs {
        warn!(
            jobs,
            floored_to = threads,
            "merkle pool: --jobs below the {MERKLE_POOL_MIN_THREADS}-thread floor required by \
             the parallel BAL merkle protocol; clamping to avoid a worker deadlock"
        );
    }
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("merkle-worker-{i}"))
            .build()
            .expect("building merkle thread pool"),
    );
    Blockchain::default_with_store_and_pool(store, pool)
}

/// Open a cold RocksDB backend: fresh block cache, optional O_DIRECT, statistics
/// on. Returned as the concrete type so `statistics_string()` is reachable.
fn open_cold_backend(db: &ColdDbArgs) -> Result<Arc<RocksDBBackend>> {
    let opts = RocksDbOpenOpts {
        block_cache_size: db.block_cache_bytes,
        use_direct_reads: db.direct_reads,
        enable_statistics: true,
    };
    let backend = RocksDBBackend::open(&db.datadir, opts)
        .with_context(|| format!("opening cold RocksDB at {}", db.datadir.display()))?;
    Ok(Arc::new(backend))
}

/// Sum of `gas_used` across every block; the numerator of the Ggas/s figure.
fn total_gas(blocks: &[Block]) -> u128 {
    blocks.iter().map(|b| b.header.gas_used as u128).sum()
}

// =============================================================================
// _warmup
// =============================================================================

pub async fn warmup(args: WarmupArgs, jobs: usize) -> Result<()> {
    info!(datadir = %args.db.datadir.display(), jobs, "warmup: capturing undo log");
    let genesis = load_genesis(&args.genesis)?;
    let (blocks, bals) = load_workload(&args.chain, &args.bals)?;

    // Concrete backend kept for the pristine/post-undo digest + direct undo
    // application; the recorder wraps it below the Store's write buffering.
    let concrete = open_cold_backend(&args.db)?;
    let recording = RecordingBackend::new(concrete.clone() as Arc<dyn StorageBackend>);

    // Build the store FIRST, then capture the pristine digest. `build_store`
    // (`set_chain_config` + `load_initial_state`) writes deterministic
    // bookkeeping markers to `misc_values` with recording OFF (chain config is
    // not persisted across reopen, and the head anchor writes a flushed-upto
    // marker). Those writes are outside the undo log, so the pristine baseline
    // must be taken AFTER them: every subprocess that opens the datadir replays
    // the exact same setup, and `_reset` restores the datadir to precisely this
    // post-setup state (the undo log covers only the recorded import writes).
    let store = build_store(
        recording.clone() as Arc<dyn StorageBackend>,
        &args.db.datadir,
        &genesis,
    )
    .await?;
    let pristine = state_digest(&*concrete).context("computing pristine state digest")?;
    std::fs::write(&args.pristine_digest, hex_encode(&pristine)).with_context(|| {
        format!(
            "writing pristine digest to {}",
            args.pristine_digest.display()
        )
    })?;

    let blockchain = build_blockchain(store.clone(), jobs);

    recording.set_recording(true);
    // Recording stays ON across the whole import + finalize so the undo log
    // captures pre-images for the SAME writes `_measure` performs — including the
    // rolling commits that flush trie/flat-KV layers to disk during the loop.
    // Otherwise reset could not restore those CFs and the digest would drift.
    import_loop(&blockchain, &store, &blocks, &bals).await?;
    finalize_head(&store, &blocks).await?;
    recording.set_recording(false);

    let undo = recording.take_undo_log();
    info!(entries = undo.len(), "warmup: captured undo log");
    save_undo_log(&undo, &args.undo_log)
        .with_context(|| format!("saving undo log to {}", args.undo_log.display()))?;

    // Restore pristine by replaying the log directly on the backend, then prove
    // the state is byte-identical to the post-setup baseline above.
    apply_undo_log(&*concrete, &undo).context("replaying undo log to restore pristine")?;
    let after = state_digest(&*concrete).context("computing post-undo state digest")?;
    if after != pristine {
        bail!(
            "warmup: post-undo state digest {} != pristine {}; the undo log does not restore \
             pristine state — reset would drift",
            hex_encode(&after),
            hex_encode(&pristine)
        );
    }
    info!("warmup: undo restores pristine (digest matches); datadir is clean for measurement");
    drop(blockchain);
    drop(store);
    Ok(())
}

// =============================================================================
// _measure
// =============================================================================

pub async fn measure(args: MeasureArgs, jobs: usize) -> Result<()> {
    info!(
        run = args.run_index,
        datadir = %args.db.datadir.display(),
        jobs,
        "measure: cold import"
    );
    let genesis = load_genesis(&args.genesis)?;
    let (blocks, bals) = load_workload(&args.chain, &args.bals)?;

    // Plain backend (NO RecordingBackend): recording pre-image reads would
    // pollute the cold-read measurement.
    let backend = open_cold_backend(&args.db)?;
    let store = build_store(
        backend.clone() as Arc<dyn StorageBackend>,
        &args.db.datadir,
        &genesis,
    )
    .await?;

    // FKV must be complete: `from_backend` expands the on-disk `[0xff]` marker to
    // `vec![0xff; 64]`. If it is not complete, cold reads would take the trie
    // path instead of the flat-KV path and the numbers would be meaningless.
    let last_written = store
        .last_written()
        .context("reading last_written cursor")?;
    // A complete FKV cursor is the on-disk `[0xff]` sentinel expanded to a full
    // 64-nibble `vec![0xff; 64]`; anything else means the index is partial.
    if last_written.len() != 64 || !last_written.iter().all(|b| *b == 0xff) {
        bail!(
            "flat-KV not complete on this datadir (last_written cursor len {} is not 64 bytes of \
             0xff); re-run gen-state so the flat-KV index is fully generated before measuring",
            last_written.len()
        );
    }

    let blockchain = build_blockchain(store.clone(), jobs);

    // Stats baseline (after open, before the timed window).
    let base = Stats::capture(&backend)?;

    // --- TIMED WINDOW --------------------------------------------------------
    let loop_start = Instant::now();
    import_loop(&blockchain, &store, &blocks, &bals).await?;
    let loop_seconds = loop_start.elapsed().as_secs_f64();

    let commit_start = Instant::now();
    finalize_head(&store, &blocks).await?;
    let commit_seconds = commit_start.elapsed().as_secs_f64();
    // --- END TIMED WINDOW ----------------------------------------------------

    let total_seconds = loop_seconds + commit_seconds;
    let after = Stats::capture(&backend)?;
    let delta = after.delta(&base);

    let gas = total_gas(&blocks);
    let ggas = if total_seconds > 0.0 {
        gas as f64 / total_seconds / 1e9
    } else {
        0.0
    };

    // Stable, greppable metrics line (Phase 6 parses this).
    let line = format!(
        "run={} jobs={} total_seconds={:.6} loop_seconds={:.6} commit_seconds={:.6} \
         ggas={:.6} block_cache_miss={} block_cache_hit={} bytes_read={} sst_read_count={}",
        args.run_index,
        jobs,
        total_seconds,
        loop_seconds,
        commit_seconds,
        ggas,
        delta.cache_miss,
        delta.cache_hit,
        delta.bytes_read,
        delta.sst_reads,
    );
    append_line(&args.out_log, &line)?;
    info!(%line, "measure: emitted metrics");

    // --- Coldness self-check (Task 5.7) --------------------------------------
    // Fail loud and name the ineffective control if any strong cold signal is at
    // or below its floor: that means reads were served warm (cache not fresh,
    // O_DIRECT not applied, or OS page cache retained).
    let mut cold_failures = Vec::new();
    if delta.cache_miss <= MIN_CACHE_MISS {
        cold_failures.push(format!(
            "block_cache_miss delta {} <= floor {} — the RocksDB block cache was not cold \
             (fresh {}-byte cache expected to miss on cold reads)",
            delta.cache_miss, MIN_CACHE_MISS, args.db.block_cache_bytes
        ));
    }
    if delta.bytes_read <= MIN_BYTES_READ {
        cold_failures.push(format!(
            "bytes_read delta {} <= floor {} — far too little data was read from Get(); the \
             workload may not be exercising cold state at all",
            delta.bytes_read, MIN_BYTES_READ
        ));
    }
    if delta.sst_reads <= MIN_SST_READS {
        cold_failures.push(format!(
            "sst_read_count delta {} <= floor {} — no SST file reads were issued; O_DIRECT \
             (direct_reads={}) and/or the OS page cache made reads warm",
            delta.sst_reads, MIN_SST_READS, args.db.direct_reads
        ));
    }
    if !cold_failures.is_empty() {
        bail!(
            "coldness self-check FAILED for run {}: {}",
            args.run_index,
            cold_failures.join("; ")
        );
    }
    info!(
        cache_miss = delta.cache_miss,
        bytes_read = delta.bytes_read,
        sst_reads = delta.sst_reads,
        "measure: coldness self-check passed"
    );

    drop(blockchain);
    drop(store);
    Ok(())
}

// =============================================================================
// _reset (undo mode)
// =============================================================================

pub async fn reset(args: ResetArgs) -> Result<()> {
    info!(datadir = %args.db.datadir.display(), "reset: replaying undo log");
    // Plain backend: replay the log directly onto the column families (no Store).
    let backend = open_cold_backend(&args.db)?;
    let undo = load_undo_log(&args.undo_log)
        .with_context(|| format!("loading undo log from {}", args.undo_log.display()))?;
    apply_undo_log(&*backend, &undo).context("replaying undo log")?;

    let pristine = read_digest(&args.pristine_digest)?;
    let after = state_digest(&*backend).context("computing post-undo state digest")?;
    if after != pristine {
        bail!(
            "reset: post-undo state digest {} != pristine {}; the datadir did not return to \
             pristine state, so the next run would measure a corrupted fixture",
            hex_encode(&after),
            hex_encode(&pristine)
        );
    }
    info!("reset: state restored to pristine (digest matches)");
    Ok(())
}

// =============================================================================
// Parent orchestration
// =============================================================================

pub async fn run_parent(args: RunArgs, jobs: usize) -> Result<()> {
    // Resolve inputs to absolute paths so re-exec'd children (which inherit the
    // parent's cwd, but for robustness we make them absolute) resolve identically.
    let datadir = absolute(&args.datadir)?;
    let chain = absolute(&args.chain)?;
    let bals = absolute(&args.bals)?;
    let genesis = absolute(&args.genesis)?;
    let out_log = absolute(&args.out_log)?;

    if args.runs == 0 {
        bail!("--runs must be >= 1");
    }

    // Validate the workload decodes and Amsterdam BAL counts match up-front, so a
    // malformed workload fails before spawning any child.
    let _ = load_workload(&chain, &bals)?;
    let _ = load_genesis(&genesis)?;

    // Working directory for the undo log + pristine digest. Under the system temp
    // dir (respects TMPDIR); removed at the end.
    let workdir = std::env::temp_dir().join(format!("state-bench-run-{}", std::process::id()));
    std::fs::create_dir_all(&workdir)
        .with_context(|| format!("creating workdir {}", workdir.display()))?;
    let undo_log = workdir.join("undo.bin");
    let pristine_digest = workdir.join("pristine.digest");

    // Fresh out-log for this invocation so the summary reflects only these runs.
    std::fs::write(&out_log, b"")
        .with_context(|| format!("truncating out-log {}", out_log.display()))?;

    info!(
        runs = args.runs,
        reset = ?args.reset,
        direct_reads = args.direct_reads,
        block_cache_bytes = args.block_cache_bytes,
        jobs,
        out_log = %out_log.display(),
        workdir = %workdir.display(),
        "run: starting cold-state benchmark"
    );

    let result = orchestrate(
        &args,
        jobs,
        &datadir,
        &chain,
        &bals,
        &genesis,
        &out_log,
        &undo_log,
        &pristine_digest,
        &workdir,
    );

    // Always clean up the workdir.
    let _ = std::fs::remove_dir_all(&workdir);

    if let Err(err) = &result {
        match args.reset {
            ResetMode::Undo => warn!(
                datadir = %datadir.display(),
                "run FAILED in undo mode: the failing child ran (or partially ran) an import \
                 directly on --datadir and the follow-up _reset was skipped, so the datadir is \
                 likely left DIRTY (post-import, non-pristine). Do NOT reuse it — regenerate it \
                 with `gen-state` before retrying. Error: {err:#}"
            ),
            ResetMode::Checkpoint => warn!(
                datadir = %datadir.display(),
                "run FAILED in checkpoint mode: the original --datadir was NOT mutated (children \
                 run on per-run copies of a checkpoint), so it is still pristine and can be \
                 reused directly. Error: {err:#}"
            ),
        }
    }
    result?;

    summarize(&out_log)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn orchestrate(
    args: &RunArgs,
    jobs: usize,
    datadir: &Path,
    chain: &Path,
    bals: &Path,
    genesis: &Path,
    out_log: &Path,
    undo_log: &Path,
    pristine_digest: &Path,
    workdir: &Path,
) -> Result<()> {
    let db_flags = |datadir: &Path| -> Vec<OsString> {
        vec![
            "--datadir".into(),
            datadir.into(),
            "--block-cache-bytes".into(),
            args.block_cache_bytes.to_string().into(),
            "--direct-reads".into(),
            args.direct_reads.to_string().into(),
        ]
    };

    match args.reset {
        ResetMode::Undo => {
            // Warmup once: capture the undo log + pristine digest, operating on the
            // pristine datadir and restoring it before exit.
            let mut warmup_args = db_flags(datadir);
            warmup_args.extend([
                "--chain".into(),
                chain.into(),
                "--bals".into(),
                bals.into(),
                "--genesis".into(),
                genesis.into(),
                "--undo-log".into(),
                undo_log.into(),
                "--pristine-digest".into(),
                pristine_digest.into(),
            ]);
            spawn_child(jobs, "_warmup", &warmup_args)?;

            for i in 1..=args.runs {
                if args.drop_caches {
                    drop_caches();
                }
                let mut measure_args = db_flags(datadir);
                measure_args.extend([
                    "--chain".into(),
                    chain.into(),
                    "--bals".into(),
                    bals.into(),
                    "--genesis".into(),
                    genesis.into(),
                    "--run-index".into(),
                    i.to_string().into(),
                    "--out-log".into(),
                    out_log.into(),
                ]);
                spawn_child(jobs, "_measure", &measure_args)?;

                let mut reset_args = db_flags(datadir);
                reset_args.extend([
                    "--undo-log".into(),
                    undo_log.into(),
                    "--pristine-digest".into(),
                    pristine_digest.into(),
                ]);
                spawn_child(jobs, "_reset", &reset_args)?;
            }
        }
        ResetMode::Checkpoint => {
            // Snapshot the pristine datadir once via a RocksDB checkpoint. A bare
            // backend (no Store) has no persist/FKV background threads retaining
            // the lock, so opening + dropping it here releases cleanly; children
            // never touch the original datadir (they run on per-run copies), so
            // there is no lock contention regardless.
            let snapshot = workdir.join("snapshot");
            make_checkpoint(datadir, &snapshot)?;

            for i in 1..=args.runs {
                if args.drop_caches {
                    drop_caches();
                }
                let run_dir = workdir.join(format!("run-{i}"));
                // Fresh copy of the snapshot for this run; SST files are hardlinked
                // (RocksDB never mutates an existing SST in place), the small
                // mutable files (MANIFEST/CURRENT/OPTIONS/LOG/WAL) + metadata.json
                // are copied so the import's appends stay private to this copy.
                hardlink_or_copy_dir(&snapshot, &run_dir).with_context(|| {
                    format!(
                        "materializing per-run datadir copy {} from {}",
                        run_dir.display(),
                        snapshot.display()
                    )
                })?;

                let mut measure_args = db_flags(&run_dir);
                measure_args.extend([
                    "--chain".into(),
                    chain.into(),
                    "--bals".into(),
                    bals.into(),
                    "--genesis".into(),
                    genesis.into(),
                    "--run-index".into(),
                    i.to_string().into(),
                    "--out-log".into(),
                    out_log.into(),
                ]);
                spawn_child(jobs, "_measure", &measure_args)?;

                // Delete the per-run copy so disk use stays bounded.
                let _ = std::fs::remove_dir_all(&run_dir);
            }
        }
    }
    Ok(())
}

/// Re-exec this binary with a hidden internal subcommand and wait for it. The
/// child inherits stdio + env (so `RUST_LOG` / tracing config carries over) and
/// receives the resolved `--jobs`. Any non-zero exit bails the whole run.
fn spawn_child(jobs: usize, subcmd: &str, args: &[OsString]) -> Result<()> {
    let exe = std::env::current_exe().context("resolving current executable path")?;
    let mut cmd = Command::new(&exe);
    cmd.arg("--jobs").arg(jobs.to_string());
    cmd.arg(subcmd);
    cmd.args(args);
    info!(subcmd, "spawning subprocess");
    let status = cmd
        .status()
        .with_context(|| format!("spawning child subprocess {subcmd}"))?;
    if !status.success() {
        bail!(
            "child subprocess {subcmd} exited with status {status}; aborting run (see child logs above)"
        );
    }
    Ok(())
}

/// Best-effort OS page-cache drop. Requires privilege; warns and continues on
/// failure (a run without page-cache eviction is still valid — RocksDB's own
/// block cache is always cold per subprocess).
fn drop_caches() {
    // Flush dirty pages first so the drop is effective.
    let _ = Command::new("sync").status();
    if std::fs::write("/proc/sys/vm/drop_caches", "3").is_ok() {
        info!("drop_caches: evicted OS page cache");
        return;
    }
    // Try an escalated, non-interactive write.
    let escalated = Command::new("sudo")
        .args(["-n", "sh", "-c", "echo 3 > /proc/sys/vm/drop_caches"])
        .status();
    match escalated {
        Ok(s) if s.success() => info!("drop_caches: evicted OS page cache via sudo"),
        _ => warn!(
            "drop_caches: could not write /proc/sys/vm/drop_caches (needs root); continuing \
             without OS page-cache eviction — RocksDB block cache is still cold per subprocess"
        ),
    }
}

/// Create a RocksDB checkpoint of `src` at `dst`, then copy the `metadata.json`
/// sidecar (a checkpoint copies only RocksDB files, not our sidecar, but
/// `from_backend_bench` requires it) plus the gen-state manifest for parity.
fn make_checkpoint(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        std::fs::remove_dir_all(dst)
            .with_context(|| format!("removing stale checkpoint {}", dst.display()))?;
    }
    {
        let backend = RocksDBBackend::open(src, RocksDbOpenOpts::default())
            .with_context(|| format!("opening {} to checkpoint", src.display()))?;
        backend
            .create_checkpoint(dst)
            .with_context(|| format!("creating checkpoint at {}", dst.display()))?;
        // Bare backend dropped here -> RocksDB lock released before children run.
    }
    // Copy non-RocksDB sidecars the checkpoint does not include.
    for sidecar in [
        ethrex_storage::STORE_METADATA_FILENAME,
        crate::manifest::MANIFEST_FILENAME,
    ] {
        let from = src.join(sidecar);
        if from.exists() {
            std::fs::copy(&from, dst.join(sidecar))
                .with_context(|| format!("copying {sidecar} into checkpoint"))?;
        }
    }
    info!(src = %src.display(), dst = %dst.display(), "checkpoint: created pristine snapshot");
    Ok(())
}

/// Copy a datadir for a checkpoint run: hardlink immutable `.sst` files (space +
/// speed), plain-copy everything else so RocksDB's in-place appends to the small
/// mutable files stay private to the copy.
fn hardlink_or_copy_dir(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        std::fs::remove_dir_all(dst)
            .with_context(|| format!("removing stale copy dir {}", dst.display()))?;
    }
    std::fs::create_dir_all(dst).with_context(|| format!("creating copy dir {}", dst.display()))?;
    for entry in
        std::fs::read_dir(src).with_context(|| format!("reading source dir {}", src.display()))?
    {
        let entry = entry.with_context(|| format!("reading dir entry in {}", src.display()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry
            .file_type()
            .with_context(|| format!("stat'ing {}", from.display()))?
            .is_dir()
        {
            hardlink_or_copy_dir(&from, &to)?;
        } else if from.extension().is_some_and(|e| e == "sst") {
            // Immutable SST: hardlink. Fall back to copy across filesystems.
            if std::fs::hard_link(&from, &to).is_err() {
                std::fs::copy(&from, &to)
                    .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
            }
        } else {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// Parse the metrics log and print mean/median of `total_seconds` and `ggas`.
fn summarize(out_log: &Path) -> Result<()> {
    let text = std::fs::read_to_string(out_log)
        .with_context(|| format!("reading out-log {}", out_log.display()))?;
    let mut totals = Vec::new();
    let mut ggas = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if let Some(v) = parse_kv_f64(line, "total_seconds") {
            totals.push(v);
        }
        if let Some(v) = parse_kv_f64(line, "ggas") {
            ggas.push(v);
        }
    }
    if totals.is_empty() {
        warn!("summary: no metrics lines found in out-log");
        return Ok(());
    }
    info!(
        runs = totals.len(),
        total_seconds_mean = mean(&totals),
        total_seconds_median = median(&totals),
        ggas_mean = mean(&ggas),
        ggas_median = median(&ggas),
        "run: summary"
    );
    Ok(())
}

// =============================================================================
// Small utilities
// =============================================================================

/// A snapshot of the RocksDB stat tickers we care about, and their deltas.
struct Stats {
    cache_miss: u64,
    cache_hit: u64,
    bytes_read: u64,
    sst_reads: u64,
}

impl Stats {
    /// Read the live statistics dump and pull out the four tickers.
    ///
    /// The dump format (RocksDB `StatisticsImpl::ToString`) is one line per
    /// ticker: `"<name> COUNT : <n>"`. `sst_read_count` sums the last-level and
    /// non-last-level SST file read tickers, which increment on every physical
    /// block read from an SST regardless of `max_open_files` (unlike
    /// `rocksdb.no.file.opens`, which fires only when a file is first opened and
    /// so stays ~flat when all readers are preloaded at open time).
    fn capture(backend: &RocksDBBackend) -> Result<Self> {
        let dump = backend
            .statistics_string()
            .context("statistics_string returned None (statistics not enabled at open time)")?;
        Ok(Self {
            cache_miss: parse_ticker(&dump, "rocksdb.block.cache.miss"),
            cache_hit: parse_ticker(&dump, "rocksdb.block.cache.hit"),
            bytes_read: parse_ticker(&dump, "rocksdb.bytes.read"),
            sst_reads: parse_ticker(&dump, "rocksdb.last.level.read.count")
                + parse_ticker(&dump, "rocksdb.non.last.level.read.count"),
        })
    }

    fn delta(&self, base: &Stats) -> Stats {
        Stats {
            cache_miss: self.cache_miss.saturating_sub(base.cache_miss),
            cache_hit: self.cache_hit.saturating_sub(base.cache_hit),
            bytes_read: self.bytes_read.saturating_sub(base.bytes_read),
            sst_reads: self.sst_reads.saturating_sub(base.sst_reads),
        }
    }
}

/// Extract the `COUNT` value of a RocksDB ticker line by exact name. Matches
/// `"<name> "` (trailing space) so a ticker is not confused with a longer name
/// that shares its prefix.
fn parse_ticker(dump: &str, name: &str) -> u64 {
    let prefix = format!("{name} ");
    for line in dump.lines() {
        if let Some(rest) = line.strip_prefix(&prefix)
            && let Some(idx) = rest.find("COUNT : ")
        {
            return rest[idx + "COUNT : ".len()..].trim().parse().unwrap_or(0);
        }
    }
    0
}

/// Parse a `key=<f64>` token out of a metrics line.
fn parse_kv_f64(line: &str, key: &str) -> Option<f64> {
    let needle = format!("{key}=");
    line.split_whitespace()
        .find_map(|tok| tok.strip_prefix(&needle))
        .and_then(|v| v.parse().ok())
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening out-log {} for append", path.display()))?;
    writeln!(f, "{line}").with_context(|| format!("appending to out-log {}", path.display()))?;
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn read_digest(path: &Path) -> Result<[u8; 32]> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading pristine digest {}", path.display()))?;
    let text = text.trim();
    if text.len() != 64 {
        bail!("pristine digest {} is not 64 hex chars", path.display());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .with_context(|| format!("parsing hex digest byte {i}"))?;
    }
    Ok(out)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let cwd = std::env::current_dir().context("resolving current directory")?;
        Ok(cwd.join(path))
    }
}
