//! Trie performance comparison benchmark
//!
//! Compares ethrex_db's MerkleTrie against:
//! - ethrex-trie (the official ethrex trie)
//! - cita_trie (used as baseline by ethrex)
//!
//! Run with: cargo bench --bench trie_comparison

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

// ethrex_db
use ethrex_db::merkle::{keccak256, MerkleTrie};

// ethrex-trie
use ethrex_trie::{Nibbles, Trie as EthrexTrie, TrieError};

// cita_trie
use cita_trie::{MemoryDB, PatriciaTrie, Trie as CitaTrie};
use hasher::HasherKeccak;

/// In-memory database for ethrex-trie (uses Mutex for Send + Sync)
/// Matches the pattern used in ethrex's own InMemoryTrieDB
#[derive(Default)]
struct InMemoryTrieDB {
    data: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl ethrex_trie::TrieDB for InMemoryTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(self
            .data
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(key.as_ref())
            .cloned())
    }

    fn put(&self, key: Nibbles, value: Vec<u8>) -> Result<(), TrieError> {
        self.data
            .lock()
            .map_err(|_| TrieError::LockError)?
            .insert(key.into_vec(), value);
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut data = self.data.lock().map_err(|_| TrieError::LockError)?;
        for (key, value) in key_values {
            data.insert(key.into_vec(), value);
        }
        Ok(())
    }
}

/// Generate test data: keys and values
fn generate_test_data(count: usize) -> Vec<([u8; 32], Vec<u8>)> {
    (0..count)
        .map(|i| {
            let key = keccak256(&(i as u64).to_be_bytes());
            let value = vec![i as u8; 32];
            (key, value)
        })
        .collect()
}

/// Benchmark insert operations
fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("Trie Insert");

    for size in [100, 1000, 10000].iter() {
        let test_data = generate_test_data(*size);
        group.throughput(Throughput::Elements(*size as u64));

        // ethrex_db MerkleTrie
        group.bench_with_input(
            BenchmarkId::new("ethrex_db", size),
            &test_data,
            |b, data| {
                b.iter(|| {
                    let mut trie = MerkleTrie::new();
                    for (key, value) in data {
                        trie.insert(key, value.clone());
                    }
                    trie.root_hash()
                })
            },
        );

        // ethrex-trie
        group.bench_with_input(
            BenchmarkId::new("ethrex_trie", size),
            &test_data,
            |b, data| {
                b.iter(|| {
                    let db = InMemoryTrieDB::default();
                    let mut trie = EthrexTrie::new(Box::new(db));
                    for (key, value) in data {
                        let _ = trie.insert(key.to_vec(), value.clone());
                    }
                    trie.hash()
                })
            },
        );

        // cita_trie
        group.bench_with_input(
            BenchmarkId::new("cita_trie", size),
            &test_data,
            |b, data| {
                b.iter(|| {
                    let memdb = Arc::new(MemoryDB::new(true));
                    let hasher = Arc::new(HasherKeccak::new());
                    let mut trie = PatriciaTrie::new(memdb, hasher);
                    for (key, value) in data {
                        let _ = trie.insert(key.to_vec(), value.clone());
                    }
                    trie.root()
                })
            },
        );
    }

    group.finish();
}

/// Benchmark get operations
fn bench_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("Trie Get");
    let test_data = generate_test_data(1000);

    // Setup tries
    let mut ethrex_db_trie = MerkleTrie::new();
    for (key, value) in &test_data {
        ethrex_db_trie.insert(key, value.clone());
    }

    let db = InMemoryTrieDB::default();
    let mut ethrex_trie = EthrexTrie::new(Box::new(db));
    for (key, value) in &test_data {
        let _ = ethrex_trie.insert(key.to_vec(), value.clone());
    }

    let memdb = Arc::new(MemoryDB::new(true));
    let hasher = Arc::new(HasherKeccak::new());
    let mut cita = PatriciaTrie::new(memdb, hasher);
    for (key, value) in &test_data {
        let _ = cita.insert(key.to_vec(), value.clone());
    }

    let lookup_key = test_data[500].0;

    group.bench_function("ethrex_db", |b| {
        b.iter(|| ethrex_db_trie.get(black_box(&lookup_key)))
    });

    group.bench_function("ethrex_trie", |b| {
        b.iter(|| ethrex_trie.get(black_box(&lookup_key)))
    });

    group.bench_function("cita_trie", |b| {
        b.iter(|| cita.get(black_box(&lookup_key.to_vec())))
    });

    group.finish();
}

/// Benchmark root hash computation
fn bench_root_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("Trie Root Hash");

    for size in [100, 1000].iter() {
        let test_data = generate_test_data(*size);

        // Setup tries (inserts included, measuring hash computation)
        let mut ethrex_db_trie = MerkleTrie::new();
        for (key, value) in &test_data {
            ethrex_db_trie.insert(key, value.clone());
        }

        let db = InMemoryTrieDB::default();
        let mut ethrex_trie = EthrexTrie::new(Box::new(db));
        for (key, value) in &test_data {
            let _ = ethrex_trie.insert(key.to_vec(), value.clone());
        }

        group.bench_with_input(BenchmarkId::new("ethrex_db", size), &(), |b, _| {
            b.iter(|| ethrex_db_trie.root_hash())
        });

        group.bench_with_input(BenchmarkId::new("ethrex_trie", size), &(), |b, _| {
            b.iter(|| ethrex_trie.hash())
        });
    }

    group.finish();
}

criterion_group!(benches, bench_insert, bench_get, bench_root_hash);
criterion_main!(benches);
