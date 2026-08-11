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
use super::group::{GroupDepth, GroupRow, MAX_GROUP_DEPTH, group_root, relative_bits};
use super::node::{StoredNode, blake3_hash, decode, encode_branch, encode_leaf};
use super::path::BitPath;
use crate::error::BinaryTrieError;
use std::collections::BTreeMap;
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
                inner: harness_db(map),
                log: log.clone(),
            },
            log,
        )
    }
}

impl BinaryTrieDB for RecordingDB {
    fn group_depth(&self) -> GroupDepth {
        self.inner.group_depth()
    }

    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        let start = Instant::now();
        let got = self.inner.get_group(group_root);
        let elapsed = start.elapsed();
        let mut log = self.log.lock().expect("log");
        log.reads += 1;
        log.read_time += elapsed;
        got
    }

    fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        self.log
            .lock()
            .expect("log")
            .written
            .extend(rows.iter().cloned());
        self.inner.put_groups(rows)
    }
}

/// A backend that accepts writes and drops them, so a commit can be
/// timed with the backend's own cost taken out.
struct SinkDB(InMemoryBinaryTrieDB);

impl BinaryTrieDB for SinkDB {
    fn group_depth(&self) -> GroupDepth {
        self.0.group_depth()
    }

    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        self.0.get_group(group_root)
    }

    fn put_groups(&self, _rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
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
    fn group_depth(&self) -> GroupDepth {
        self.inner.group_depth()
    }

    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        self.inner.get_group(group_root)
    }

    fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let mut staged = self.staged.lock().expect("staging buffer");
        staged.reserve(rows.len());
        for (group_root, encoded) in rows {
            staged.push((group_root.to_db_key(), encoded));
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

    let db = harness_db(Default::default());
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
    let scratch = harness_db(Arc::clone(map));
    let mut trie = BinaryTrie::open(Box::new(SinkDB(scratch)), root);
    apply_all(&mut trie);
    let start = Instant::now();
    trie.commit().expect("commit");
    let commit_no_write = start.elapsed();
    drop(trie);

    // Commit arm shaped like production: entries pushed into a staging
    // vector under their database keys, nothing touching disk.
    let staging = StagingDB {
        inner: harness_db(Arc::clone(map)),
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
    // Unwrapped once, up front: the timed loops below measure hashing and
    // encoding of *nodes*, which is the work `collect` does, and timing
    // them over row bytes would fold in the row container instead.
    let encodings: Vec<Vec<u8>> = written.iter().map(|(_, row)| node_bytes(row)).collect();
    let nodes: Vec<StoredNode> = encodings
        .iter()
        .map(|encoded| decode(encoded).expect("commit wrote a decodable node"))
        .collect();

    // Hashing: blake3 over each node's stored bytes.
    let start = Instant::now();
    let mut sink = 0u8;
    for _ in 0..reps {
        for encoded in &encodings {
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

/// The harness stores one node per row, so `map` stays a node-granular
/// picture and [`rows_at`] can bucket it into rows at *any* depth from
/// outside. Grouping at the store would fix the depth at build time and
/// leave the sweep unable to compare depths over one state.
const HARNESS_DEPTH: GroupDepth = match GroupDepth::new(1) {
    Some(depth) => depth,
    None => unreachable!(),
};

/// A stored backend for the harness: one node per row, so a row key is
/// a node path and a row holds exactly one member.
fn harness_db(map: NodeMap) -> InMemoryBinaryTrieDB {
    InMemoryBinaryTrieDB::new(map).at_group_depth(HARNESS_DEPTH)
}

/// The node inside a one-member row, which is what every stored value in
/// this harness is.
fn node_bytes(row: &[u8]) -> Vec<u8> {
    let row = GroupRow::decode(row).expect("harness wrote a decodable row");
    let members = row.members();
    assert_eq!(members.len(), 1, "harness rows hold exactly one node");
    assert!(members[0].0.is_empty(), "the one member is the row's root");
    members[0].1.clone()
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
            .filter(|(_, row)| matches!(decode(&node_bytes(row)), Ok(StoredNode::Leaf { .. })))
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

/// Recover the [`BitPath`] a database key was built from.
///
/// The inverse of [`BitPath::to_db_key`]: a four-byte big-endian bit
/// count, then that many bits packed MSB-first. Only the harness needs
/// it — production never goes backwards from a key — so it lives here
/// rather than on `BitPath`, and it panics on a malformed key because
/// the only keys it ever sees are ones this file just wrote.
fn path_from_db_key(key: &[u8]) -> BitPath {
    let count = u32::from_be_bytes(key[..4].try_into().expect("four-byte count")) as usize;
    let packed = &key[4..];
    BitPath::from_bits(
        &(0..count)
            .map(|i| (packed[i / 8] >> (7 - i % 8)) & 1)
            .collect::<Vec<u8>>(),
    )
}

/// Every stored node bucketed into the row it would occupy at
/// `group_depth`, keyed by the row's database key.
///
/// Built over the *whole* database, not over the dirty set, because a
/// row that a commit rewrites has to carry its untouched members too:
/// that is exactly what makes grouped writes bigger than ungrouped
/// ones, and it cannot be measured from the dirty set alone.
fn rows_at(map: &NodeMap, group_depth: GroupDepth) -> BTreeMap<Vec<u8>, GroupRow> {
    let nodes = map.lock().expect("map");
    let mut by_path: Vec<(BitPath, Vec<u8>)> = nodes
        .iter()
        .map(|(key, row)| (path_from_db_key(key), node_bytes(row)))
        .collect();
    // Sorted by (depth, bits) so members go into each row in the order
    // `GroupRow::push` demands.
    by_path.sort_by(|a, b| (a.0.len(), a.0.as_bits()).cmp(&(b.0.len(), b.0.as_bits())));

    let mut rows: BTreeMap<Vec<u8>, GroupRow> = BTreeMap::new();
    for (path, encoded) in by_path {
        let row_key = group_root(&path, group_depth).to_db_key();
        rows.entry(row_key)
            .or_default()
            .push(
                relative_bits(&path, group_depth),
                encoded.clone(),
                group_depth,
            )
            .expect("a node belongs in the row its own path selects");
    }
    rows
}

/// How grouping trades row count against row size, at every group depth
/// this crate can store, over our own key distribution.
///
/// The two numbers that matter are on the same line: **rows touched**
/// per block, which is what the 82.4%-of-block-time node lookups pay,
/// and **bytes written**, which is what grouping costs to get it.
/// go-ethereum ships `groupDepth = 5` and says openly it does not know
/// the optimum; this answers it for our workload rather than inheriting
/// the number.
#[test]
#[ignore = "measurement harness, not a correctness test"]
fn group_depth_sweep() {
    // Two state sizes, because the row count is *not* monotonic in the
    // group depth: a band boundary that lands just above the depth the
    // tree fans out at splits far more rows than one that lands just
    // below. The depth distribution moves with the leaf count, so a
    // ranking taken at one size is not evidence about another, and the
    // second size is what says whether the winner is a real optimum or
    // an artefact of where the bands happened to fall.
    for leaf_count in [100_000usize, 1_000_000] {
        sweep_at(leaf_count);
    }
}

fn sweep_at(leaf_count: usize) {
    {
        let (map, root, keys) = build(leaf_count, 0x2545F4914F6CDD1D);
        let node_count = map.lock().expect("map").len();
        println!("\n=== group depth sweep: {leaf_count} leaves -> {node_count} nodes ===");

        // Dirty sets first, so every group depth is scored on the same
        // blocks rather than on fresh random ones.
        let rounds: Vec<(usize, Vec<BitPath>, usize)> = [1usize, 100, 1_000, 5_000]
            .iter()
            .map(|&changed| {
                let r = round(&map, root, &keys, changed, 0xDEADBEEF ^ changed as u64);
                let bytes = r.written.iter().map(|(_, e)| e.len()).sum();
                (
                    changed,
                    r.written.into_iter().map(|(p, _)| p).collect(),
                    bytes,
                )
            })
            .collect();

        for levels in 1..=MAX_GROUP_DEPTH {
            let group_depth = GroupDepth::new(levels).expect("in range");
            let rows = rows_at(&map, group_depth);
            let row_bytes: Vec<usize> = rows.values().map(|row| row.encode().len()).collect();
            let total_bytes: usize = row_bytes.iter().sum();
            let max_row = row_bytes.iter().copied().max().unwrap_or(0);
            let mean_members =
                rows.values().map(|r| r.members().len()).sum::<usize>() as f64 / rows.len() as f64;
            // Rows with no member at the empty relative path. A branch
            // whose prefix jumps a band boundary puts *both* its children
            // in the next group and neither at that group's root, so a
            // group can be entered at two nodes and have no root member
            // at all. Counted rather than argued: a `resolve` that
            // assumes a root member passes every hand-written test and
            // fails on real state.
            let rootless = rows.values().filter(|row| row.get(&[]).is_none()).count();
            println!(
                "\ng={levels}  rows={:<9} ({:>5.2}x fewer)  mean members {mean_members:>5.2}  \
             mean row {:>6.0} B  max row {max_row} B  table {:>6.1} MiB  \
             rootless rows {rootless} ({:>5.2}%)",
                rows.len(),
                node_count as f64 / rows.len() as f64,
                total_bytes as f64 / rows.len() as f64,
                total_bytes as f64 / (1024.0 * 1024.0),
                100.0 * rootless as f64 / rows.len() as f64,
            );
            for (changed, dirty, flat_bytes) in &rounds {
                // Reads equal the dirty set exactly (see
                // `dirty_nodes_per_changed_leaf`), and a read of any member
                // fetches its whole row, so rows read and rows written are
                // the same set — one number, not two.
                let touched: std::collections::BTreeSet<Vec<u8>> = dirty
                    .iter()
                    .map(|path| group_root(path, group_depth).to_db_key())
                    .collect();
                let written_bytes: usize = touched
                    .iter()
                    .map(|key| rows.get(key).map_or(0, |row| row.encode().len()))
                    .sum();
                println!(
                    "  changed={changed:<5} rows touched={:<6} ({:>6.2}/leaf, {:>5.2}x fewer than \
                 {} nodes)  bytes written {:>8} ({:>5.2}x of {flat_bytes})",
                    touched.len(),
                    touched.len() as f64 / *changed as f64,
                    dirty.len() as f64 / touched.len() as f64,
                    dirty.len(),
                    written_bytes,
                    written_bytes as f64 / *flat_bytes as f64,
                );
            }
        }
    }
}
