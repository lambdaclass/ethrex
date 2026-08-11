//! TEMPORARY measurement harness: what does a binary-trie block cost
//! against a real RocksDB, and how much of it is node I/O?
//!
//! Run with:
//!   cargo test -p ethrex-storage --release --features rocksdb \
//!     binary_trie_cost -- --ignored --nocapture
//!
//! The in-memory harness in `ethrex-binary-trie` measures the CPU side
//! (encode, hash, path building). This one supplies the number that one
//! cannot: what a node read actually costs when it has to come off disk.

#![cfg(all(test, feature = "rocksdb"))]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ethrex_binary_trie::BinaryTrieError;
use ethrex_binary_trie::trie::{
    BinaryTrie, BinaryTrieDB, BitPath, DEFAULT_GROUP_DEPTH, GroupDepth, group_root,
};

use crate::api::StorageBackend;
use crate::api::tables::BINARY_TRIE_NODES;
use crate::backend::rocksdb::{RocksDBBackend, RocksDBConfig};
use crate::binary_trie::BackendBinaryTrieDB;

/// Deterministic xorshift, matching the binary-trie harness so the two
/// measure the same key distribution.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn key(&mut self) -> Vec<u8> {
        let mut key = vec![0u8; 34];
        for chunk in key[..32].chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_be_bytes()[..chunk.len()]);
        }
        key[33] = (self.next_u64() % 4) as u8;
        key
    }

    fn value(&mut self) -> [u8; 32] {
        let mut value = [0u8; 32];
        for chunk in value.chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_be_bytes()[..chunk.len()]);
        }
        value
    }
}

/// [`BackendBinaryTrieDB`] pinned to a chosen group depth.
///
/// `BackendBinaryTrieDB` takes [`BinaryTrieDB::group_depth`]'s default,
/// which is whatever `DEFAULT_GROUP_DEPTH` happens to be. Deciding
/// between two candidate depths by wall clock means running the *same*
/// build against a real RocksDB at each of them, so the depth has to be
/// an argument here rather than a constant — otherwise comparing two
/// depths costs two rebuilds of the crate and the two runs cannot share
/// a process, a page cache or a machine state.
struct DepthDB {
    inner: BackendBinaryTrieDB,
    depth: GroupDepth,
}

impl DepthDB {
    fn new(backend: Arc<dyn StorageBackend>, depth: GroupDepth) -> Self {
        Self {
            inner: BackendBinaryTrieDB::new(backend).expect("read view"),
            depth,
        }
    }
}

impl BinaryTrieDB for DepthDB {
    fn group_depth(&self) -> GroupDepth {
        self.depth
    }

    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        self.inner.get_group(group_root)
    }

    fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        self.inner.put_groups(rows)
    }
}

/// Times reads and stages writes the way production does: `put_batch`
/// pushes into a vector, it does not touch disk. Reads go to RocksDB.
struct TimedStagingDB {
    inner: DepthDB,
    reads: Mutex<(usize, Duration)>,
    staged: Mutex<Vec<(Vec<u8>, Vec<u8>)>>,
    stage_time: Mutex<Duration>,
}

impl TimedStagingDB {
    fn new(backend: Arc<dyn StorageBackend>, depth: GroupDepth) -> Self {
        Self {
            inner: DepthDB::new(backend, depth),
            reads: Mutex::new((0, Duration::ZERO)),
            staged: Mutex::new(Vec::new()),
            stage_time: Mutex::new(Duration::ZERO),
        }
    }
}

impl BinaryTrieDB for TimedStagingDB {
    fn group_depth(&self) -> GroupDepth {
        self.inner.group_depth()
    }

    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        let start = Instant::now();
        let got = self.inner.get_group(group_root);
        let elapsed = start.elapsed();
        let mut reads = self.reads.lock().expect("reads");
        reads.0 += 1;
        reads.1 += elapsed;
        got
    }

    fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let start = Instant::now();
        let mut staged = self.staged.lock().expect("staged");
        staged.reserve(rows.len());
        for (group_root, encoded) in rows {
            staged.push((group_root.to_db_key(), encoded));
        }
        let elapsed = start.elapsed();
        *self.stage_time.lock().expect("stage time") += elapsed;
        Ok(())
    }
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// A comma-separated list of numbers from `var`, or `fallback`.
///
/// The sweep's axes — which depths, which state sizes, how many blocks —
/// are the thing one wants to move while chasing a result, and moving
/// them by editing a literal means a recompile of `ethrex-storage`
/// against RocksDB for every question asked. Reading them from the
/// environment keeps the committed defaults as the documented run while
/// leaving the axes open.
fn numbers_from_env(var: &str, fallback: &[usize]) -> Vec<usize> {
    match std::env::var(var) {
        Err(_) => fallback.to_vec(),
        Ok(raw) => raw
            .split(',')
            .map(|part| {
                part.trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("{var}: {part:?} is not a number"))
            })
            .collect(),
    }
}

/// Build an `leaf_count`-leaf binary trie into `backend`, returning the
/// root and the keys.
fn build(
    backend: Arc<dyn StorageBackend>,
    leaf_count: usize,
    seed: u64,
    depth: GroupDepth,
) -> (ethrex_common::H256, Vec<Vec<u8>>) {
    let mut rng = Rng(seed);
    let mut leaves: Vec<(Vec<u8>, [u8; 32])> =
        (0..leaf_count).map(|_| (rng.key(), rng.value())).collect();
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    leaves.dedup_by(|a, b| a.0 == b.0);
    let keys: Vec<Vec<u8>> = leaves.iter().map(|(key, _)| key.clone()).collect();

    let db = DepthDB::new(backend, depth);
    let mut trie =
        BinaryTrie::from_sorted_leaves(Box::new(db), leaves).expect("bulk build over sorted keys");
    let root = trie.commit().expect("commit the bulk build").root;
    (root, keys)
}

/// What one block cost, so a caller can aggregate several blocks rather
/// than read them off the printed lines one at a time.
#[derive(Clone, Copy, Default)]
struct BlockCost {
    rows_written: usize,
    row_reads: usize,
    read_time: Duration,
    /// Reads made by `commit`, not by `apply`.
    ///
    /// `BinaryTrie::build_rows` re-reads a row whose *stored* members it
    /// must carry forward — an all-removals row, or one the dirty set
    /// entered below its root. Those are `get_group` calls like any
    /// other and the original harness folded them into a figure it
    /// printed as a percentage of `apply`, which overstates the apply
    /// share by exactly this much. Split out rather than left implicit.
    commit_reads: usize,
    commit_read_time: Duration,
    apply: Duration,
    commit: Duration,
}

impl BlockCost {
    fn total(&self) -> Duration {
        self.apply + self.commit
    }
}

/// One block's worth of work against a RocksDB-backed trie.
fn round(
    backend: Arc<dyn StorageBackend>,
    root: ethrex_common::H256,
    keys: &[Vec<u8>],
    changed: usize,
    seed: u64,
    depth: GroupDepth,
    report: bool,
) -> BlockCost {
    let mut rng = Rng(seed);
    let picks: Vec<Vec<u8>> = (0..changed)
        .map(|_| keys[(rng.next_u64() as usize) % keys.len()].clone())
        .collect();
    let values: Vec<[u8; 32]> = (0..changed).map(|_| rng.value()).collect();

    let db = Arc::new(TimedStagingDB::new(backend, depth));
    // The trie takes a boxed backend, so hand it a second handle on the
    // same counters rather than moving the one holding them.
    struct Handle(Arc<TimedStagingDB>);
    impl BinaryTrieDB for Handle {
        fn group_depth(&self) -> GroupDepth {
            self.0.group_depth()
        }
        fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
            self.0.get_group(group_root)
        }
        fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
            self.0.put_groups(rows)
        }
    }

    let mut trie = BinaryTrie::open(Box::new(Handle(db.clone())), root);
    let start = Instant::now();
    for (key, value) in picks.iter().zip(&values) {
        trie.insert(key.clone(), *value).expect("insert");
    }
    let apply = start.elapsed();
    // Snapshotted between the phases: `commit` reads too, and folding
    // its reads into the apply figure is what made the old line report
    // read shares of `apply` above what `apply` ever spent reading.
    let (apply_reads, apply_read_time) = *db.reads.lock().expect("reads");
    let start = Instant::now();
    trie.commit().expect("commit");
    let commit = start.elapsed();

    let (read_count, read_time) = *db.reads.lock().expect("reads");
    let stage_time = *db.stage_time.lock().expect("stage time");
    let rows = db.staged.lock().expect("staged").len();
    let commit_reads = read_count - apply_reads;
    let commit_read_time = read_time - apply_read_time;

    if report {
        println!(
            "changed={changed:<5} rows={rows:<7} ({:>5.1}/leaf)  row_reads={read_count:<7} \
             ({apply_reads} in apply, {commit_reads} in commit)\n  \
             apply           {:>10.1} us   of which RocksDB reads {:>10.1} us ({:>4.1}%) \
             at {:>6.2} us/read\n  \
             commit          {:>10.1} us   of which RocksDB reads {:>10.1} us ({:>4.1}%), \
             staging put_groups {:>8.1} us ({:>4.1}%)\n  \
             block total     {:>10.1} us   read share of block {:>4.1}%",
            rows as f64 / changed as f64,
            micros(apply),
            micros(apply_read_time),
            100.0 * micros(apply_read_time) / micros(apply),
            micros(apply_read_time) / apply_reads.max(1) as f64,
            micros(commit),
            micros(commit_read_time),
            100.0 * micros(commit_read_time) / micros(commit),
            micros(stage_time),
            100.0 * micros(stage_time) / micros(commit),
            micros(apply) + micros(commit),
            100.0 * micros(read_time) / (micros(apply) + micros(commit)),
        );
    }

    BlockCost {
        rows_written: rows,
        row_reads: read_count,
        read_time,
        commit_reads,
        commit_read_time,
        apply,
        commit,
    }
}

fn run_at(leaf_count: usize, block_cache_size: usize, label: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend: Arc<dyn StorageBackend> = Arc::new(
        RocksDBBackend::open(
            dir.path(),
            RocksDBConfig {
                block_cache_size,
                enable_statistics: false,
            },
        )
        .expect("open rocksdb"),
    );

    let start = Instant::now();
    let (root, keys) = build(
        backend.clone(),
        leaf_count,
        0x2545F4914F6CDD1D,
        DEFAULT_GROUP_DEPTH,
    );
    backend.flush().expect("flush");
    println!(
        "\n=== {label}: {leaf_count} leaves, {} MiB block cache (built + flushed in {:.1} s) ===",
        block_cache_size / (1024 * 1024),
        start.elapsed().as_secs_f64()
    );

    control_read_cost(backend.clone(), &keys, DEFAULT_GROUP_DEPTH);

    for &changed in &[1usize, 100, 1_000] {
        round(
            backend.clone(),
            root,
            &keys,
            changed,
            0xDEADBEEF ^ changed as u64,
            DEFAULT_GROUP_DEPTH,
            true,
        );
    }
}

/// Untimed-per-call control: one `Instant` around a whole batch of raw
/// `BINARY_TRIE_NODES` lookups, so the per-read figure the round reports
/// can be checked against one that carries no timer overhead at all.
///
/// Reads leaf paths, whose depth is the deepest the trie has, so this is
/// the same kind of lookup a descent's last step makes.
fn control_read_cost(backend: Arc<dyn StorageBackend>, keys: &[Vec<u8>], depth: GroupDepth) {
    use ethrex_binary_trie::trie::BitPath;

    let db = DepthDB::new(backend, depth);
    // Paths that certainly exist are unknown without a descent, so probe
    // a fixed shallow depth where every path is populated, plus a set of
    // deep paths derived from real keys.
    let probes: Vec<BitPath> = keys
        .iter()
        .take(20_000)
        .map(|key| {
            let bits: Vec<u8> = (0..20).map(|i| (key[i / 8] >> (7 - i % 8)) & 1).collect();
            BitPath::from_bits(&bits)
        })
        .collect();

    // Keep only the paths that exist. A descent reads nothing but present
    // nodes, whereas a mixed set is diluted by misses the bloom filter
    // rejects for almost nothing — which would make the control look
    // faster than the descent for a reason that has nothing to do with
    // the descent.
    // Truncated to their group roots first: a lookup is a *row* lookup now,
    // and a 20-bit path addresses no row unless 20 is a multiple of the group
    // depth. Probing the raw paths would filter almost everything out and
    // measure a table of misses.
    let probes: Vec<BitPath> = probes
        .into_iter()
        .map(|path| group_root(&path, db.group_depth()))
        .filter(|row| db.get_group(row).expect("get_group").is_some())
        .collect();

    // `keys` is sorted, so probing it in order is a *sequential* scan and
    // reuses SST blocks. A descent is random access. Measuring both, with
    // one timer around the whole batch rather than one per call, separates
    // three things that could each explain the per-read cost a round
    // reports: instrumentation overhead, access pattern, and cache size.
    let mut shuffled = probes.clone();
    let mut rng = Rng(0xA5A5_A5A5_DEAD_BEEF);
    for i in (1..shuffled.len()).rev() {
        shuffled.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
    }

    for (label, set) in [("sequential", &probes), ("random", &shuffled)] {
        for pass in 0..2 {
            let start = Instant::now();
            let mut hits = 0usize;
            for row in set {
                if db.get_group(row).expect("get_group").is_some() {
                    hits += 1;
                }
            }
            let elapsed = start.elapsed();
            println!(
                "  control {label:<10} pass {pass}: {} raw lookups ({hits} hits) \
                 in {:>9.1} us = {:.2} us/read, no per-call timer",
                set.len(),
                micros(elapsed),
                micros(elapsed) / set.len() as f64,
            );
        }
    }
}

/// One `(state size, group depth)` cell of the wall-clock sweep.
struct DepthResult {
    depth: GroupDepth,
    table_rows: usize,
    table_bytes: usize,
    build: Duration,
    blocks: Vec<BlockCost>,
}

impl DepthResult {
    /// Median block, by total time. The median rather than the mean
    /// because one block that lands during a compaction or a scheduler
    /// hiccup moves a five-sample mean by more than the effect being
    /// measured.
    fn median_total(&self) -> Duration {
        let mut totals: Vec<Duration> = self.blocks.iter().map(BlockCost::total).collect();
        totals.sort();
        totals[totals.len() / 2]
    }

    fn mean_total(&self) -> Duration {
        self.blocks.iter().map(BlockCost::total).sum::<Duration>() / self.blocks.len() as u32
    }

    fn spread(&self) -> (Duration, Duration) {
        let mut totals: Vec<Duration> = self.blocks.iter().map(BlockCost::total).collect();
        totals.sort();
        (totals[0], totals[totals.len() - 1])
    }

    fn mean_reads(&self) -> f64 {
        self.blocks.iter().map(|b| b.row_reads).sum::<usize>() as f64 / self.blocks.len() as f64
    }

    fn mean_rows_written(&self) -> f64 {
        self.blocks.iter().map(|b| b.rows_written).sum::<usize>() as f64 / self.blocks.len() as f64
    }

    fn mean_read_time(&self) -> Duration {
        self.blocks.iter().map(|b| b.read_time).sum::<Duration>() / self.blocks.len() as u32
    }

    fn mean_commit_reads(&self) -> f64 {
        self.blocks.iter().map(|b| b.commit_reads).sum::<usize>() as f64 / self.blocks.len() as f64
    }

    fn mean_commit_read_time(&self) -> Duration {
        self.blocks
            .iter()
            .map(|b| b.commit_read_time)
            .sum::<Duration>()
            / self.blocks.len() as u32
    }

    fn mean_apply(&self) -> Duration {
        self.blocks.iter().map(|b| b.apply).sum::<Duration>() / self.blocks.len() as u32
    }

    fn mean_commit(&self) -> Duration {
        self.blocks.iter().map(|b| b.commit).sum::<Duration>() / self.blocks.len() as u32
    }
}

/// A state built at one group depth, held open and ready to be timed.
struct Store {
    depth: GroupDepth,
    /// Kept alive: dropping it deletes the RocksDB directory.
    _dir: tempfile::TempDir,
    backend: Arc<dyn StorageBackend>,
    root: ethrex_common::H256,
    keys: Vec<Vec<u8>>,
    table_rows: usize,
    table_bytes: usize,
    build: Duration,
}

/// Build a state at `depth` on a fresh RocksDB and leave it open with a
/// cold block cache.
///
/// **Reopened between the build and the measurement.** The bulk build
/// touches every row it writes, so it leaves the block cache holding
/// whatever fits — which at these state sizes is a large fraction of the
/// table, and a larger one for the depth whose hot rows happen to be
/// fewer. Timing reads against that cache measures the build's access
/// pattern, not a block's. Reopening drops the cache and the memtables
/// and leaves only SSTs, which is what a node that restarts and then
/// imports a block actually reads from.
fn prepare_store(leaf_count: usize, block_cache_size: usize, depth: GroupDepth) -> Store {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = RocksDBConfig {
        block_cache_size,
        enable_statistics: false,
    };

    let build_start = Instant::now();
    let (root, keys) = {
        let backend: Arc<dyn StorageBackend> =
            Arc::new(RocksDBBackend::open(dir.path(), config).expect("open rocksdb"));
        let built = build(backend.clone(), leaf_count, 0x2545F4914F6CDD1D, depth);
        backend.flush().expect("flush");
        built
    };
    let build = build_start.elapsed();

    // Counted off disk rather than projected: the sweep in
    // `ethrex-binary-trie` buckets a node-granular map into rows, this is
    // the row count the store really holds at this depth.
    let (table_rows, table_bytes) = {
        let backend: Arc<dyn StorageBackend> =
            Arc::new(RocksDBBackend::open(dir.path(), config).expect("reopen rocksdb"));
        let view = backend.begin_read().expect("read view");
        let mut rows = 0usize;
        let mut bytes = 0usize;
        for entry in view
            .prefix_iterator(BINARY_TRIE_NODES, &[])
            .expect("iterate rows")
        {
            let (key, value) = entry.expect("row");
            rows += 1;
            bytes += key.len() + value.len();
        }
        (rows, bytes)
    };

    // Reopen once more: the count above just scanned the whole table
    // into the cache.
    let backend: Arc<dyn StorageBackend> =
        Arc::new(RocksDBBackend::open(dir.path(), config).expect("reopen rocksdb"));

    Store {
        depth,
        _dir: dir,
        backend,
        root,
        keys,
        table_rows,
        table_bytes,
        build,
    }
}

/// Time `blocks` blocks against every store, **interleaved**, and report
/// one [`DepthResult`] per store.
///
/// Interleaved because the first arrangement of this measurement — all
/// of one depth, then all of the other — produced a 57% drift on
/// *identical* work between the first cell of the run and the last, which
/// is several times the effect being measured and which systematically
/// penalises whichever depth runs second. Alternating block by block,
/// and reversing the order on odd-numbered blocks, leaves any drift
/// spread evenly over both.
///
/// Every block is applied against its store's *own* root and its writes
/// are staged, never flushed, exactly as `LayeredBinaryTrieDB` does — so
/// the on-disk state does not drift between blocks and the depths are
/// scored on identical rows. Block `b` uses the same seed for every
/// depth, so the two depths see the same changed leaves.
fn time_interleaved(stores: &[Store], blocks: usize, changed: usize) -> Vec<DepthResult> {
    let mut costs: Vec<Vec<BlockCost>> = vec![Vec::with_capacity(blocks); stores.len()];
    for block in 0..blocks {
        let seed = 0xB10C_0000 ^ (block as u64).wrapping_mul(0x9E37_79B9);
        let mut order: Vec<usize> = (0..stores.len()).collect();
        if block % 2 == 1 {
            order.reverse();
        }
        for index in order {
            let store = &stores[index];
            costs[index].push(round(
                store.backend.clone(),
                store.root,
                &store.keys,
                changed,
                seed,
                store.depth,
                false,
            ));
        }
    }

    stores
        .iter()
        .zip(costs)
        .map(|(store, blocks)| DepthResult {
            depth: store.depth,
            table_rows: store.table_rows,
            table_bytes: store.table_bytes,
            build: store.build,
            blocks,
        })
        .collect()
}

/// **The group-depth decision, by wall clock rather than by row count.**
///
/// `g = 6` and `g = 7` were argued over projected rows touched, marginal
/// write cost and the fraction of rows with no root member — three
/// proxies that point in different directions and that no amount of
/// further arithmetic reconciles. A rootless row costs a *second read*
/// when a descent later enters it through its other entry, and that read
/// is a real RocksDB lookup here rather than a modelled one, so timing
/// the block folds all three proxies into one number.
///
/// Several state sizes because every single-size conclusion on this
/// question has so far been reversed by adding a size: the depth
/// distribution moves with the leaf count and both the row count and the
/// rootless fraction swing non-monotonically with it.
#[test]
#[ignore = "measurement harness, not a correctness test"]
fn group_depth_wall_clock() {
    const CHANGED: usize = 1_000;

    let blocks: usize = numbers_from_env("ETHREX_GD_BLOCKS", &[15])[0];
    // `g = 1` is the ungrouped control — one node per row, which is what
    // the table held before this plan — so the run says not only which
    // depth is best but whether grouping is winning at all. 5 and 8
    // bracket the two candidates: a two-point comparison cannot say
    // whether either is a local optimum, and `g = 8` is the depth the
    // projected rows-touched column liked best while criterion (b)
    // excludes it, so it is worth seeing what it does to a real block.
    let candidates: Vec<GroupDepth> = numbers_from_env("ETHREX_GD_DEPTHS", &[1, 4, 5, 6, 7, 8])
        .into_iter()
        .map(|levels| GroupDepth::new(levels).expect("in range"))
        .collect();
    let leaf_counts = numbers_from_env("ETHREX_GD_LEAVES", &[300_000, 600_000, 1_000_000]);
    let cache_sizes: Vec<usize> = numbers_from_env("ETHREX_GD_CACHE_MIB", &[8, 64])
        .into_iter()
        .map(|mib| mib * 1024 * 1024)
        .collect();

    for &leaf_count in &leaf_counts {
        for &block_cache_size in &cache_sizes {
            let cache_mib = block_cache_size / (1024 * 1024);
            println!(
                "\n\n######## group depth wall clock: {leaf_count} leaves, \
                 {cache_mib} MiB block cache, {blocks} interleaved blocks of \
                 {CHANGED} changed leaves ########"
            );

            let stores: Vec<Store> = candidates
                .iter()
                .map(|&depth| prepare_store(leaf_count, block_cache_size, depth))
                .collect();
            let results = time_interleaved(&stores, blocks, CHANGED);

            for r in &results {
                let (low, high) = r.spread();
                println!(
                    "  g={}  table {:>9} rows / {:>6.1} MiB (built {:>5.1} s)  \
                     reads/block {:>8.1} ({:>6.1} in commit)  rows written/block {:>8.1}\n        \
                     block median {:>9.1} us  mean {:>9.1} us  min {:>9.1} us  max {:>9.1} us\n        \
                     apply {:>9.1} us   commit {:>9.1} us   \
                     read time/block {:>9.1} us ({:>4.1}% of block, {:>7.1} us in commit)  \
                     {:>5.2} us/read",
                    r.depth.get(),
                    r.table_rows,
                    r.table_bytes as f64 / (1024.0 * 1024.0),
                    r.build.as_secs_f64(),
                    r.mean_reads(),
                    r.mean_commit_reads(),
                    r.mean_rows_written(),
                    micros(r.median_total()),
                    micros(r.mean_total()),
                    micros(low),
                    micros(high),
                    micros(r.mean_apply()),
                    micros(r.mean_commit()),
                    micros(r.mean_read_time()),
                    100.0 * micros(r.mean_read_time()) / micros(r.mean_total()),
                    micros(r.mean_commit_read_time()),
                    micros(r.mean_read_time()) / r.mean_reads().max(1.0),
                );
            }
            let baseline = results
                .iter()
                .find(|r| r.depth == DEFAULT_GROUP_DEPTH)
                .expect("the default depth is one of the candidates");
            for r in &results {
                if r.depth == baseline.depth {
                    continue;
                }
                println!(
                    "  >>> g={} vs g={} @ {leaf_count} leaves / {cache_mib} MiB: \
                     median block {:>+6.1}%   mean block {:>+6.1}%   apply {:>+6.1}%   \
                     commit {:>+6.1}%   reads {:>+6.1}%   read time {:>+6.1}%",
                    r.depth.get(),
                    baseline.depth.get(),
                    100.0 * (micros(r.median_total()) / micros(baseline.median_total()) - 1.0),
                    100.0 * (micros(r.mean_total()) / micros(baseline.mean_total()) - 1.0),
                    100.0 * (micros(r.mean_apply()) / micros(baseline.mean_apply()) - 1.0),
                    100.0 * (micros(r.mean_commit()) / micros(baseline.mean_commit()) - 1.0),
                    100.0 * (r.mean_reads() / baseline.mean_reads() - 1.0),
                    100.0 * (micros(r.mean_read_time()) / micros(baseline.mean_read_time()) - 1.0),
                );
            }
        }
    }
}

#[test]
#[ignore = "measurement harness, not a correctness test"]
fn binary_trie_cost_on_rocksdb() {
    // Small cache: reads must reach SSTs, which is what a mainnet-sized
    // node looks like. Large cache: everything is resident, which is what
    // the 5 MB devnet against a 12 GiB cache was measuring.
    run_at(1_000_000, 64 * 1024 * 1024, "cold-ish");
    run_at(1_000_000, 8 * 1024 * 1024 * 1024, "fully cached");
}
