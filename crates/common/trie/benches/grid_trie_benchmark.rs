//! Benchmarks comparing grid trie vs recursive trie performance.
//!
//! Run with: cargo bench --features grid-trie

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ethereum_types::H256;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_trie::{db::InMemoryTrieDB, grid::HexPatriciaGrid, Trie};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Generate test data with n keys
fn generate_test_data(n: u32) -> Vec<(H256, Vec<u8>)> {
    let mut data: Vec<_> = (0..n)
        .map(|i| {
            let key = i.to_be_bytes();
            let hashed = H256::from_slice(&keccak_hash(&key));
            let value = key.to_vec();
            (hashed, value)
        })
        .collect();
    data.sort_by_key(|(k, _)| *k);
    data
}

/// Benchmark recursive trie insertion
fn bench_recursive_trie(data: &[(H256, Vec<u8>)]) -> H256 {
    let mut trie = Trie::new_temp();
    for (key, value) in data {
        trie.insert(key.as_bytes().to_vec(), value.clone()).unwrap();
    }
    trie.hash_no_commit()
}

/// Benchmark grid trie insertion
fn bench_grid_trie(data: &[(H256, Vec<u8>)]) -> H256 {
    let db = InMemoryTrieDB::new(Arc::new(Mutex::new(BTreeMap::new())));
    let mut grid = HexPatriciaGrid::new(db);
    grid.apply_sorted_updates(data.iter().cloned()).unwrap()
}

fn trie_comparison_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie_comparison");

    for size in [100, 500, 1000, 2000, 5000] {
        let data = generate_test_data(size);

        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(BenchmarkId::new("recursive", size), &data, |b, data| {
            b.iter(|| bench_recursive_trie(black_box(data)))
        });

        group.bench_with_input(BenchmarkId::new("grid", size), &data, |b, data| {
            b.iter(|| bench_grid_trie(black_box(data)))
        });
    }

    group.finish();
}

fn grid_trie_scaling_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("grid_scaling");

    for size in [100, 250, 500, 750, 1000, 1500, 2000] {
        let data = generate_test_data(size);

        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(BenchmarkId::new("grid_trie", size), &data, |b, data| {
            b.iter(|| bench_grid_trie(black_box(data)))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    trie_comparison_benchmark,
    grid_trie_scaling_benchmark
);
criterion_main!(benches);
