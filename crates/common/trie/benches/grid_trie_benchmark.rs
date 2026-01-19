//! Benchmarks comparing grid trie vs recursive trie performance.
//!
//! Run with: cargo bench --features grid-trie

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ethereum_types::H256;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_trie::{
    db::InMemoryTrieDB,
    grid::{ConcurrentPatriciaGrid, HexPatriciaGrid},
    Trie,
};
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

/// Benchmark sequential grid trie insertion
fn bench_grid_trie(data: &[(H256, Vec<u8>)]) -> H256 {
    let db = InMemoryTrieDB::new(Arc::new(Mutex::new(BTreeMap::new())));
    let mut grid = HexPatriciaGrid::new(db);
    grid.apply_sorted_updates(data.iter().cloned()).unwrap()
}

/// Benchmark parallel grid trie insertion
fn bench_concurrent_grid_trie(data: &[(H256, Vec<u8>)]) -> H256 {
    let db = InMemoryTrieDB::new(Arc::new(Mutex::new(BTreeMap::new())));
    let mut grid = ConcurrentPatriciaGrid::new(db);
    grid.apply_sorted_updates_parallel(data.iter().cloned())
        .unwrap()
}

fn trie_comparison_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie_comparison");

    for size in [100, 500, 1000, 2000, 5000] {
        let data = generate_test_data(size);

        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(BenchmarkId::new("recursive", size), &data, |b, data| {
            b.iter(|| bench_recursive_trie(black_box(data)))
        });

        group.bench_with_input(BenchmarkId::new("grid_seq", size), &data, |b, data| {
            b.iter(|| bench_grid_trie(black_box(data)))
        });

        group.bench_with_input(BenchmarkId::new("grid_parallel", size), &data, |b, data| {
            b.iter(|| bench_concurrent_grid_trie(black_box(data)))
        });
    }

    group.finish();
}

fn parallel_scaling_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_scaling");

    for size in [500, 1000, 2000, 5000, 10000] {
        let data = generate_test_data(size);

        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(
            BenchmarkId::new("grid_sequential", size),
            &data,
            |b, data| b.iter(|| bench_grid_trie(black_box(data))),
        );

        group.bench_with_input(
            BenchmarkId::new("grid_parallel", size),
            &data,
            |b, data| b.iter(|| bench_concurrent_grid_trie(black_box(data))),
        );
    }

    group.finish();
}

fn large_scale_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_scale");
    group.sample_size(10); // Reduce sample size for large datasets

    for size in [100000, 500000, 1000000, 4000000] {
        let data = generate_test_data(size);

        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(BenchmarkId::new("recursive", size), &data, |b, data| {
            b.iter(|| bench_recursive_trie(black_box(data)))
        });

        group.bench_with_input(BenchmarkId::new("grid_seq", size), &data, |b, data| {
            b.iter(|| bench_grid_trie(black_box(data)))
        });

        group.bench_with_input(BenchmarkId::new("grid_parallel", size), &data, |b, data| {
            b.iter(|| bench_concurrent_grid_trie(black_box(data)))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    trie_comparison_benchmark,
    parallel_scaling_benchmark,
    large_scale_benchmark
);
criterion_main!(benches);
