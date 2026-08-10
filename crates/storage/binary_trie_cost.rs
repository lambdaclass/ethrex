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

use ethrex_binary_trie::trie::{BinaryTrie, BinaryTrieDB, BitPath};
use ethrex_binary_trie::BinaryTrieError;

use crate::api::StorageBackend;
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

/// Times reads and stages writes the way production does: `put_batch`
/// pushes into a vector, it does not touch disk. Reads go to RocksDB.
struct TimedStagingDB {
    inner: BackendBinaryTrieDB,
    reads: Mutex<(usize, Duration)>,
    staged: Mutex<Vec<(Vec<u8>, Vec<u8>)>>,
    stage_time: Mutex<Duration>,
}

impl TimedStagingDB {
    fn new(backend: Arc<dyn StorageBackend>) -> Self {
        Self {
            inner: BackendBinaryTrieDB::new(backend).expect("read view"),
            reads: Mutex::new((0, Duration::ZERO)),
            staged: Mutex::new(Vec::new()),
            stage_time: Mutex::new(Duration::ZERO),
        }
    }
}

impl BinaryTrieDB for TimedStagingDB {
    fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        let start = Instant::now();
        let got = self.inner.get(path);
        let elapsed = start.elapsed();
        let mut reads = self.reads.lock().expect("reads");
        reads.0 += 1;
        reads.1 += elapsed;
        got
    }

    fn put_batch(&self, entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let start = Instant::now();
        let mut staged = self.staged.lock().expect("staged");
        staged.reserve(entries.len());
        for (path, encoded) in entries {
            staged.push((path.to_db_key(), encoded));
        }
        let elapsed = start.elapsed();
        *self.stage_time.lock().expect("stage time") += elapsed;
        Ok(())
    }
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// Build an `leaf_count`-leaf binary trie into `backend`, returning the
/// root and the keys.
fn build(
    backend: Arc<dyn StorageBackend>,
    leaf_count: usize,
    seed: u64,
) -> (ethrex_common::H256, Vec<Vec<u8>>) {
    let mut rng = Rng(seed);
    let mut leaves: Vec<(Vec<u8>, [u8; 32])> =
        (0..leaf_count).map(|_| (rng.key(), rng.value())).collect();
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    leaves.dedup_by(|a, b| a.0 == b.0);
    let keys: Vec<Vec<u8>> = leaves.iter().map(|(key, _)| key.clone()).collect();

    let db = BackendBinaryTrieDB::new(backend).expect("read view");
    let mut trie =
        BinaryTrie::from_sorted_leaves(Box::new(db), leaves).expect("bulk build over sorted keys");
    let root = trie.commit().expect("commit the bulk build").root;
    (root, keys)
}

/// One block's worth of work against a RocksDB-backed trie.
fn round(
    backend: Arc<dyn StorageBackend>,
    root: ethrex_common::H256,
    keys: &[Vec<u8>],
    changed: usize,
    seed: u64,
) {
    let mut rng = Rng(seed);
    let picks: Vec<Vec<u8>> = (0..changed)
        .map(|_| keys[(rng.next_u64() as usize) % keys.len()].clone())
        .collect();
    let values: Vec<[u8; 32]> = (0..changed).map(|_| rng.value()).collect();

    let db = Arc::new(TimedStagingDB::new(backend));
    // The trie takes a boxed backend, so hand it a second handle on the
    // same counters rather than moving the one holding them.
    struct Handle(Arc<TimedStagingDB>);
    impl BinaryTrieDB for Handle {
        fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
            self.0.get(path)
        }
        fn put_batch(&self, entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
            self.0.put_batch(entries)
        }
    }

    let mut trie = BinaryTrie::open(Box::new(Handle(db.clone())), root);
    let start = Instant::now();
    for (key, value) in picks.iter().zip(&values) {
        trie.insert(key.clone(), *value).expect("insert");
    }
    let apply = start.elapsed();
    let start = Instant::now();
    trie.commit().expect("commit");
    let commit = start.elapsed();

    let (read_count, read_time) = *db.reads.lock().expect("reads");
    let stage_time = *db.stage_time.lock().expect("stage time");
    let dirty = db.staged.lock().expect("staged").len();

    println!(
        "changed={changed:<5} dirty={dirty:<7} ({:>5.1}/leaf)  node_reads={read_count:<7}\n  \
         apply           {:>10.1} us   of which RocksDB reads {:>10.1} us ({:>4.1}%) \
         at {:>6.2} us/read\n  \
         commit          {:>10.1} us   of which staging put_batch {:>8.1} us ({:>4.1}%)\n  \
         block total     {:>10.1} us   read share of block {:>4.1}%",
        dirty as f64 / changed as f64,
        micros(apply),
        micros(read_time),
        100.0 * micros(read_time) / micros(apply),
        micros(read_time) / read_count.max(1) as f64,
        micros(commit),
        micros(stage_time),
        100.0 * micros(stage_time) / micros(commit),
        micros(apply) + micros(commit),
        100.0 * micros(read_time) / (micros(apply) + micros(commit)),
    );
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
    let (root, keys) = build(backend.clone(), leaf_count, 0x2545F4914F6CDD1D);
    backend.flush().expect("flush");
    println!(
        "\n=== {label}: {leaf_count} leaves, {} MiB block cache (built + flushed in {:.1} s) ===",
        block_cache_size / (1024 * 1024),
        start.elapsed().as_secs_f64()
    );

    for &changed in &[1usize, 100, 1_000] {
        round(
            backend.clone(),
            root,
            &keys,
            changed,
            0xDEADBEEF ^ changed as u64,
        );
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
