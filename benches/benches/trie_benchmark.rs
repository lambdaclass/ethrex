//! Microbenchmarks for the Merkle-Patricia-Trie (`ethrex-trie`).
//!
//! These exercise only the public, stable `Trie` API (`new`/`open`/`get`/`insert`/
//! `remove`/`hash`/`hash_no_commit`) so they keep measuring the same thing across
//! internal refactors of the trie (node layout, nibble representation, and so on).
//!
//! The shape of the trie matters more than its size: keys are 32-byte pseudorandom
//! hashes, exactly like the keccak-hashed account addresses and storage slots the
//! real state trie is keyed by. That makes the trie wide (branch nodes with many
//! children near the root) and shallow (depth ~7 at this population), which is the
//! regime where per-traversal-level work dominates.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use ethrex_common::{H256, NativeCrypto};
use ethrex_trie::{InMemoryTrieDB, Trie, db::NodeMap};

/// Number of leaves in the benchmark trie.
///
/// 50k keeps the whole suite well under a couple of minutes while still giving a
/// realistic depth: with 32-byte uniformly distributed keys, 50k leaves means the
/// first ~4 nibbles are dense branch nodes and leaves sit around depth 6-7. Going to
/// 100k+ only adds ~1 level, at a large cost in fixture build time and memory.
const TRIE_SIZE: usize = 50_000;

/// Keys touched per `insert` / `remove` / `root_hash` iteration.
///
/// Roughly the number of state-trie updates a small block produces, and large enough
/// that a single iteration is comfortably above criterion's timing resolution.
const BATCH_KEYS: usize = 64;

/// Value size in bytes: about the size of an RLP-encoded `AccountState`, so leaf
/// encoding and hashing costs are representative.
const VALUE_SIZE: usize = 72;

const POPULATED_SEED: u64 = 0x1234_5678_9abc_def0;
const MISSING_SEED: u64 = 0x0fed_cba9_8765_4321;
const FRESH_SEED: u64 = 0xdead_beef_cafe_f00d;

/// SplitMix64. Deterministic and dependency-free, with good enough avalanche that the
/// 32-byte keys it produces are indistinguishable (for trie-shape purposes) from
/// keccak output. Determinism is the point: before/after numbers must compare.
struct SplitMix64(u64);

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            out.extend_from_slice(&self.next_u64().to_be_bytes());
        }
        out.truncate(len);
        out
    }
}

/// Deterministic `(key, value)` pairs drawn from a single seeded stream.
fn kv_pairs(seed: u64, count: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut rng = SplitMix64::new(seed);
    (0..count)
        .map(|_| (rng.next_bytes(32), rng.next_bytes(VALUE_SIZE)))
        .collect()
}

/// A committed trie plus the key sets the benchmarks probe it with.
///
/// The trie is only ever handed out via [`Fixture::open`], which rebuilds a `Trie`
/// whose root is a bare `NodeRef::Hash`. Reads therefore go through `TrieDB` node
/// decoding on every level instead of hitting an already-materialised
/// `NodeRef::Node` graph, which is what a node actually does when serving state.
struct Fixture {
    nodes: NodeMap,
    root: H256,
    /// Keys present in the trie.
    present: Vec<Vec<u8>>,
    /// Keys guaranteed (with overwhelming probability) not to be in the trie.
    missing: Vec<Vec<u8>>,
    /// Pairs to insert; disjoint from `present`.
    fresh: Vec<(Vec<u8>, Vec<u8>)>,
    /// Subset of `present`, spread across the key space, used by `remove`.
    to_remove: Vec<Vec<u8>>,
}

impl Fixture {
    fn build() -> Self {
        let nodes: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
        let mut trie = Trie::new(Box::new(InMemoryTrieDB::new(Arc::clone(&nodes))));

        let pairs = kv_pairs(POPULATED_SEED, TRIE_SIZE);
        for (key, value) in &pairs {
            trie.insert(key.clone(), value.clone())
                .expect("failed to build the benchmark trie");
        }
        // Flush every node into the in-memory DB and take the root hash, so the trie
        // can be re-opened cold from `nodes` for each measurement.
        let root = trie
            .hash(&NativeCrypto)
            .expect("failed to commit the benchmark trie");

        let present: Vec<Vec<u8>> = pairs.into_iter().map(|(key, _)| key).collect();
        // Sample the removals with a stride across the whole key list rather than
        // taking a contiguous prefix, so they are spread over the trie instead of
        // clustering under a handful of top-level branches.
        let stride = present.len() / BATCH_KEYS;
        let to_remove = present
            .iter()
            .step_by(stride.max(1))
            .take(BATCH_KEYS)
            .cloned()
            .collect();

        let fixture = Self {
            nodes,
            root,
            present,
            missing: kv_pairs(MISSING_SEED, BATCH_KEYS * 16)
                .into_iter()
                .map(|(key, _)| key)
                .collect(),
            fresh: kv_pairs(FRESH_SEED, BATCH_KEYS),
            to_remove,
        };
        fixture.sanity_check();
        fixture
    }

    /// Guard against silently benchmarking nothing: a `get_hit` that always misses, or
    /// a `get_miss` that hits, would measure a truncated descent and look like a win.
    fn sanity_check(&self) {
        let trie = self.open();
        for key in self.present.iter().step_by(self.present.len() / 8) {
            assert!(
                trie.get(key).expect("get must not fail").is_some(),
                "a key inserted into the fixture is not readable back"
            );
        }
        for key in &self.missing {
            assert!(
                trie.get(key).expect("get must not fail").is_none(),
                "a `missing` key is actually present in the fixture"
            );
        }
        for (key, _) in &self.fresh {
            assert!(
                trie.get(key).expect("get must not fail").is_none(),
                "a `fresh` key is already present in the fixture"
            );
        }
    }

    /// Re-open the committed trie. Cheap: it only wraps a clone of the shared node
    /// map and the root hash, so every iteration starts from the identical state.
    ///
    /// `Trie` is not `Clone`, so this replaces the "clone the base trie per batch"
    /// pattern. It is equivalent as long as the routine does not `commit`, which none
    /// of the mutating benchmarks below do: `insert` and `remove` only read from the
    /// node map, leaving the fixture pristine.
    fn open(&self) -> Trie {
        Trie::open(
            Box::new(InMemoryTrieDB::new(Arc::clone(&self.nodes))),
            self.root,
        )
    }
}

fn trie_benchmark(c: &mut Criterion) {
    let fixture = Fixture::build();
    let mut group = c.benchmark_group("trie");

    // Lookups of keys that exist: a full root-to-leaf descent, one DB read per level.
    // This is the dominant cost of state access, and the case a traversal-path
    // refactor should move.
    //
    // Caveat: `Trie::get` takes `&self`, so it cannot memoize the root it decodes.
    // Every call therefore re-decodes the root branch node on top of the descent.
    // That cost is constant across refactors, so the comparison stays valid, it just
    // dilutes the per-level signal a little.
    group.bench_function("get_hit", |b| {
        let trie = fixture.open();
        let mut i = 0usize;
        b.iter(|| {
            let key = &fixture.present[i % fixture.present.len()];
            i = i.wrapping_add(1);
            black_box(
                trie.get(black_box(key.as_slice()))
                    .expect("get on a committed trie must not fail"),
            )
        });
    });

    // Lookups of absent keys: the descent stops early, at the first branch node with
    // no child for the next nibble.
    group.bench_function("get_miss", |b| {
        let trie = fixture.open();
        let mut i = 0usize;
        b.iter(|| {
            let key = &fixture.missing[i % fixture.missing.len()];
            i = i.wrapping_add(1);
            black_box(
                trie.get(black_box(key.as_slice()))
                    .expect("get on a committed trie must not fail"),
            )
        });
    });

    // `Trie::insert` takes owned `Vec`s, so the key/value clones are hoisted into the
    // untimed setup closure to keep them out of the measurement.
    group.bench_function("insert", |b| {
        b.iter_batched(
            || (fixture.open(), fixture.fresh.clone()),
            |(mut trie, fresh)| {
                for (key, value) in fresh {
                    trie.insert(key, value).expect("insert must not fail");
                }
                trie
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("remove", |b| {
        b.iter_batched(
            || fixture.open(),
            |mut trie| {
                for key in &fixture.to_remove {
                    black_box(trie.remove(key).expect("remove must not fail"));
                }
                trie
            },
            BatchSize::SmallInput,
        );
    });

    // Root-hash computation over a dirty trie, i.e. what happens at the end of block
    // execution. `hash_no_commit` is used rather than `hash` so the measurement is
    // merkleization only, without the DB write batch. The dirtying inserts happen in
    // the untimed setup; nodes untouched by them keep their cached hashes, exactly as
    // in a real incremental state-root update.
    group.bench_function("root_hash", |b| {
        b.iter_batched(
            || {
                let mut trie = fixture.open();
                for (key, value) in &fixture.fresh {
                    trie.insert(key.clone(), value.clone())
                        .expect("insert must not fail");
                }
                trie
            },
            |trie| black_box(trie.hash_no_commit(&NativeCrypto)),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn trie_criterion() -> Criterion {
    Criterion::default()
        .sample_size(50)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
}

criterion_group!(
    name = trie_bench;
    config = trie_criterion();
    targets = trie_benchmark
);
criterion_main!(trie_bench);
