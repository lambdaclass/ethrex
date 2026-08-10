//! TEMPORARY measurement harness: where does binary-trie commit time go?
//!
//! Run with:
//!   cargo test -p ethrex-binary-trie --release commit_cost -- --ignored --nocapture
//!
//! Not a correctness test. Every number it prints is a wall-clock
//! measurement over the real node multiset a commit produces.

use std::time::{Duration, Instant};

use super::binary_trie::BinaryTrie;
use super::db::{BinaryTrieDB, InMemoryBinaryTrieDB, NodeMap};
use super::node::{StoredNode, blake3_hash, decode, encode_branch, encode_leaf};
use super::path::BitPath;
use crate::error::BinaryTrieError;
use std::sync::{Arc, Mutex};

/// Deterministic xorshift, so runs are comparable.
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

    /// A 34-byte tree key: 32-byte stem plus a one-byte suffix.
    fn key(&mut self) -> Vec<u8> {
        let mut key = vec![0u8; 34];
        for chunk in key[..32].chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_be_bytes()[..chunk.len()]);
        }
        key[32] = 0;
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

/// Records everything a commit hands to the backend, and counts the
/// reads the *apply* phase made, separately.
#[derive(Default)]
struct Recorded {
    reads: usize,
    read_time: Duration,
    written: Vec<(BitPath, Vec<u8>)>,
}

struct RecordingDB {
    inner: InMemoryBinaryTrieDB,
    log: Arc<Mutex<Recorded>>,
}

impl RecordingDB {
    fn over(map: NodeMap) -> (Self, Arc<Mutex<Recorded>>) {
        let log = Arc::new(Mutex::new(Recorded::default()));
        (
            Self {
                inner: InMemoryBinaryTrieDB::new(map),
                log: log.clone(),
            },
            log,
        )
    }
}

impl BinaryTrieDB for RecordingDB {
    fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        let start = Instant::now();
        let got = self.inner.get(path);
        let elapsed = start.elapsed();
        let mut log = self.log.lock().expect("log");
        log.reads += 1;
        log.read_time += elapsed;
        got
    }

    fn put_batch(&self, entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        self.log
            .lock()
            .expect("log")
            .written
            .extend(entries.iter().cloned());
        self.inner.put_batch(entries)
    }
}

/// A backend that accepts writes and drops them, so a commit can be
/// timed with the backend's own cost taken out.
struct SinkDB(InMemoryBinaryTrieDB);

impl BinaryTrieDB for SinkDB {
    fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        self.0.get(path)
    }

    fn put_batch(&self, _entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        Ok(())
    }
}

/// What production actually does with a commit's entries: push them
/// into an in-memory staging vector under their database keys.
///
/// Mirrors `LayeredBinaryTrieDB::put_batch` in `crates/storage`, which
/// writes nothing to disk — the staged nodes reach RocksDB later, at the
/// layer flush. The in-memory backend's `put_batch` is a `BTreeMap`
/// insert instead, which is *not* the production shape and costs far
/// more, so timing a commit against it overstates the write.
struct StagingDB {
    inner: InMemoryBinaryTrieDB,
    staged: StagedNodes,
}

/// The staging buffer's type, matching `StagedBinaryNodes` in `crates/storage`.
type StagedNodes = Arc<Mutex<Vec<(Vec<u8>, Vec<u8>)>>>;

impl BinaryTrieDB for StagingDB {
    fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        self.inner.get(path)
    }

    fn put_batch(&self, entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let mut staged = self.staged.lock().expect("staging buffer");
        staged.reserve(entries.len());
        for (path, encoded) in entries {
            staged.push((path.to_db_key(), encoded));
        }
        Ok(())
    }
}

/// Build a trie of `leaf_count` random leaves and return its backing
/// map, its root, and the keys, so a later round can update a subset.
fn build(leaf_count: usize, seed: u64) -> (NodeMap, ethereum_types::H256, Vec<Vec<u8>>) {
    let mut rng = Rng(seed);
    let mut leaves: Vec<(Vec<u8>, [u8; 32])> = (0..leaf_count)
        .map(|_| (rng.key(), rng.value()))
        .collect::<Vec<_>>();
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    leaves.dedup_by(|a, b| a.0 == b.0);

    let db = InMemoryBinaryTrieDB::new_empty();
    let map = db.inner();
    let mut trie = BinaryTrie::from_sorted_leaves(Box::new(db), leaves.clone())
        .expect("bulk build over sorted unique keys");
    let root = trie.commit().expect("commit the bulk build").root;
    (map, root, leaves.into_iter().map(|(key, _)| key).collect())
}

/// One block's worth of work against an already-built trie: reopen it
/// cold (every node a stored reference, as production does per block),
/// update `changed` keys, and commit.
struct Round {
    apply: Duration,
    read_time: Duration,
    reads: usize,
    commit_no_write: Duration,
    commit_staging: Duration,
    written: Vec<(BitPath, Vec<u8>)>,
}

fn round(
    map: &NodeMap,
    root: ethereum_types::H256,
    keys: &[Vec<u8>],
    changed: usize,
    seed: u64,
) -> Round {
    let mut rng = Rng(seed);
    let picks: Vec<Vec<u8>> = (0..changed)
        .map(|_| keys[(rng.next_u64() as usize) % keys.len()].clone())
        .collect();
    let values: Vec<[u8; 32]> = (0..changed).map(|_| rng.value()).collect();

    let apply_all = |trie: &mut BinaryTrie| {
        for (key, value) in picks.iter().zip(&values) {
            trie.insert(key.clone(), *value).expect("insert");
        }
    };

    // Capture arm, untimed for commit: the recorder clones every entry,
    // so its commit time is not reportable. It exists only to hand back
    // the exact node multiset and the read count.
    let (db, log) = RecordingDB::over(Arc::clone(map));
    let mut trie = BinaryTrie::open(Box::new(db), root);
    apply_all(&mut trie);
    trie.commit().expect("commit");
    drop(trie);
    let recorded = std::mem::take(&mut *log.lock().expect("log"));

    // Apply arm: reads timed inside the backend, so the read share of
    // apply is separated from the tree-walking around it.
    let (db, log) = RecordingDB::over(Arc::clone(map));
    let mut trie = BinaryTrie::open(Box::new(db), root);
    let start = Instant::now();
    apply_all(&mut trie);
    let apply = start.elapsed();
    let read_time = log.lock().expect("log").read_time;
    drop(trie);

    // Commit arm with no backend at all: the trie's own commit cost —
    // recursion, encoding, hashing, path building, mark_clean.
    let scratch = InMemoryBinaryTrieDB::new(Arc::clone(map));
    let mut trie = BinaryTrie::open(Box::new(SinkDB(scratch)), root);
    apply_all(&mut trie);
    let start = Instant::now();
    trie.commit().expect("commit");
    let commit_no_write = start.elapsed();
    drop(trie);

    // Commit arm shaped like production: entries pushed into a staging
    // vector under their database keys, nothing touching disk.
    let staging = StagingDB {
        inner: InMemoryBinaryTrieDB::new(Arc::clone(map)),
        staged: Arc::new(Mutex::new(Vec::new())),
    };
    let mut trie = BinaryTrie::open(Box::new(staging), root);
    apply_all(&mut trie);
    let start = Instant::now();
    trie.commit().expect("commit");
    let commit_staging = start.elapsed();

    Round {
        apply,
        read_time,
        reads: recorded.reads,
        commit_no_write,
        commit_staging,
        written: recorded.written,
    }
}

/// Time hashing and encoding over exactly the nodes a commit wrote.
///
/// Both are pure functions of their inputs, so replaying them over the
/// decoded real nodes measures the same work `collect` did, without a
/// per-node timer distorting it.
fn replay(written: &[(BitPath, Vec<u8>)], reps: u32) -> (Duration, Duration, Duration, Duration) {
    let nodes: Vec<StoredNode> = written
        .iter()
        .map(|(_, encoded)| decode(encoded).expect("commit wrote a decodable node"))
        .collect();

    // Hashing: blake3 over each node's stored bytes.
    let start = Instant::now();
    let mut sink = 0u8;
    for _ in 0..reps {
        for (_, encoded) in written {
            sink ^= blake3_hash(std::hint::black_box(encoded)).0[0];
        }
    }
    let hash = start.elapsed() / reps;

    // Encoding: node fields back to stored bytes.
    let start = Instant::now();
    for _ in 0..reps {
        for node in &nodes {
            let encoded = match node {
                StoredNode::Leaf { key, value } => encode_leaf(key, value),
                StoredNode::Branch {
                    prefix,
                    left,
                    right,
                } => encode_branch(prefix, *left, *right),
            };
            sink ^= std::hint::black_box(&encoded)[0];
        }
    }
    let encode = start.elapsed() / reps;

    // Database keys: what put_batch derives from each path.
    let start = Instant::now();
    for _ in 0..reps {
        for (path, _) in written {
            sink ^= std::hint::black_box(path.to_db_key())[0];
        }
    }
    let db_key = start.elapsed() / reps;

    // Path construction: the two child paths a branch builds on the way
    // down. Replayed as one `child` call per written node, which is what
    // `collect` makes: every node below the root is reached by exactly one.
    let start = Instant::now();
    for _ in 0..reps {
        for (path, _) in written {
            let bits = path.as_bits();
            let split = bits.len().saturating_sub(1);
            let parent = BitPath::from_bits(&bits[..split]);
            sink ^= std::hint::black_box(parent.child(&[], *bits.last().unwrap_or(&0))).len() as u8;
        }
    }
    let path_build = start.elapsed() / reps;

    std::hint::black_box(sink);
    (hash, encode, db_key, path_build)
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

#[test]
#[ignore = "measurement harness, not a correctness test"]
fn commit_cost_breakdown() {
    for &leaf_count in &[10_000usize, 100_000, 1_000_000] {
        let build_start = Instant::now();
        let (map, root, keys) = build(leaf_count, 0x2545F4914F6CDD1D);
        let node_count = map.lock().expect("map").len();
        println!(
            "\n=== state: {leaf_count} leaves -> {node_count} nodes (built in {:.1} s) ===",
            build_start.elapsed().as_secs_f64()
        );

        for &changed in &[1usize, 100, 1_000, 5_000] {
            let r = round(&map, root, &keys, changed, 0xDEADBEEF ^ changed as u64);
            let reps = if r.written.len() > 20_000 { 20 } else { 200 };
            let (hash, encode, db_key, path_build) = replay(&r.written, reps);
            let dirty = r.written.len();
            let core = micros(r.commit_no_write);
            let residual = core - micros(hash) - micros(encode) - micros(path_build);
            println!(
                "changed={changed:<5} dirty={dirty:<6} ({:.1}/leaf)  node_reads={:<6}\n  \
                 apply           {:>9.1} us  (of which backend reads {:>8.1} us)\n  \
                 commit  core    {:>9.1} us   +staging {:>8.1} us (staging share {:>7.1} us)\n  \
                 core split: hash {:>8.1} us ({:>4.1}%) | encode {:>7.1} us ({:>4.1}%) \
                 | path {:>7.1} us ({:>4.1}%) | residual {:>7.1} us ({:>4.1}%)\n  \
                 staging replay: db_key {:>7.1} us",
                dirty as f64 / changed as f64,
                r.reads,
                micros(r.apply),
                micros(r.read_time),
                core,
                micros(r.commit_staging),
                micros(r.commit_staging) - core,
                micros(hash),
                100.0 * micros(hash) / core,
                micros(encode),
                100.0 * micros(encode) / core,
                micros(path_build),
                100.0 * micros(path_build) / core,
                residual,
                100.0 * residual / core,
                micros(db_key),
            );
        }
        drop(keys);
    }
}

/// How many nodes one changed leaf dirties, and how that falls as a
/// block touches more leaves and they share upper path.
#[test]
#[ignore = "measurement harness, not a correctness test"]
fn dirty_nodes_per_changed_leaf() {
    let (map, root, keys) = build(1_000_000, 0x2545F4914F6CDD1D);
    println!("\n=== dirty nodes per changed leaf, 1M-leaf state ===");
    for &changed in &[1usize, 2, 5, 10, 50, 100, 500, 1_000, 5_000, 20_000] {
        let r = round(&map, root, &keys, changed, 0x1234 ^ changed as u64);
        let leaves = r
            .written
            .iter()
            .filter(|(_, encoded)| matches!(decode(encoded), Ok(StoredNode::Leaf { .. })))
            .count();
        let depths: Vec<usize> = r.written.iter().map(|(path, _)| path.len()).collect();
        let max_depth = depths.iter().copied().max().unwrap_or(0);
        let mean_depth = depths.iter().sum::<usize>() as f64 / depths.len().max(1) as f64;
        println!(
            "changed={changed:<6} dirty={:<7} ({:>6.2} per leaf)  of which leaves={leaves:<6} \
             branches={:<7} reads={:<7} mean_depth={mean_depth:>6.1} max_depth={max_depth}",
            r.written.len(),
            r.written.len() as f64 / changed as f64,
            r.written.len() - leaves,
            r.reads,
        );
    }
}
