//! Criterion benchmarks for snap sync healing cache
//!
//! These benchmarks provide statistical analysis of healing cache performance
//! with proper confidence intervals and PR comparison support.
//!
//! Run locally: `cargo bench --bench healing_benchmark`
//! CI comparison: Uses boa-dev/criterion-compare-action

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use ethrex_p2p::sync::healing_cache::{HealingCache, PathStatus};
use ethrex_trie::Nibbles;
use std::sync::Arc;
use std::thread;

/// Generate deterministic test paths for benchmarking
fn generate_test_paths(count: usize, depth: usize) -> Vec<Nibbles> {
    (0..count)
        .map(|i| {
            let bytes: Vec<u8> = (0..depth)
                .map(|j| ((i * 17 + j * 31) % 256) as u8)
                .collect();
            Nibbles::from_bytes(&bytes)
        })
        .collect()
}

/// Generate paths that are different from the standard test paths (for false positive testing)
fn generate_unknown_paths(count: usize, depth: usize) -> Vec<Nibbles> {
    (0..count)
        .map(|i| {
            let bytes: Vec<u8> = (0..depth)
                .map(|j| ((i * 19 + j * 37 + 12345) % 256) as u8)
                .collect();
            Nibbles::from_bytes(&bytes)
        })
        .collect()
}

// ===== CACHE SINGLE OPERATION BENCHMARKS =====

fn bench_cache_check_single(c: &mut Criterion) {
    let cache = HealingCache::new();
    let paths = generate_test_paths(10_000, 32);

    // Pre-populate half the paths
    cache.mark_exists_batch(&paths[..5000]);

    c.bench_function("cache_check_single", |b| {
        let mut idx = 0;
        b.iter(|| {
            let status = cache.check_path(&paths[idx % paths.len()]);
            idx += 1;
            black_box(status)
        })
    });
}

fn bench_cache_mark_single(c: &mut Criterion) {
    let paths = generate_test_paths(10_000, 32);

    c.bench_function("cache_mark_single", |b| {
        let cache = HealingCache::new();
        let mut idx = 0;
        b.iter(|| {
            cache.mark_exists(&paths[idx % paths.len()]);
            idx += 1;
        })
    });
}

// ===== BATCH OPERATION BENCHMARKS =====

fn bench_cache_batch_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_batch");

    for size in [100, 1_000, 10_000].iter() {
        let paths = generate_test_paths(*size, 32);

        group.throughput(Throughput::Elements(*size as u64));

        // Benchmark batch mark
        group.bench_with_input(BenchmarkId::new("mark", size), size, |b, _| {
            let cache = HealingCache::new();
            b.iter(|| cache.mark_exists_batch(black_box(&paths)))
        });

        // Benchmark batch check (with half pre-populated)
        let cache = HealingCache::new();
        cache.mark_exists_batch(&paths[..*size / 2]);

        group.bench_with_input(BenchmarkId::new("check", size), size, |b, _| {
            b.iter(|| black_box(cache.check_paths_batch(&paths)))
        });
    }

    group.finish();
}

// ===== CACHE STATE BENCHMARKS =====

fn bench_cache_empty_vs_warm(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_state");
    let paths = generate_test_paths(10_000, 32);

    // Empty cache benchmark
    group.bench_function("empty_cache_check", |b| {
        let cache = HealingCache::new();
        let mut idx = 0;
        b.iter(|| {
            let status = cache.check_path(&paths[idx % paths.len()]);
            idx += 1;
            black_box(status)
        })
    });

    // Warm cache benchmark (all paths exist)
    let warm_cache = HealingCache::new();
    warm_cache.mark_exists_batch(&paths);

    group.bench_function("warm_cache_check", |b| {
        let mut idx = 0;
        b.iter(|| {
            let status = warm_cache.check_path(&paths[idx % paths.len()]);
            idx += 1;
            black_box(status)
        })
    });

    // Partial cache benchmark (50% hit rate)
    let partial_cache = HealingCache::new();
    partial_cache.mark_exists_batch(&paths[..5000]);

    group.bench_function("partial_cache_check", |b| {
        let mut idx = 0;
        b.iter(|| {
            let status = partial_cache.check_path(&paths[idx % paths.len()]);
            idx += 1;
            black_box(status)
        })
    });

    group.finish();
}

// ===== CONCURRENT ACCESS BENCHMARKS =====

fn bench_cache_concurrent(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_concurrent");

    for num_threads in [2, 4, 8].iter() {
        let cache = Arc::new(HealingCache::new());
        let paths = generate_test_paths(10_000, 32);

        // Pre-populate
        cache.mark_exists_batch(&paths);

        group.throughput(Throughput::Elements(10_000));
        group.bench_with_input(
            BenchmarkId::new("read_threads", num_threads),
            num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let handles: Vec<_> = (0..num_threads)
                        .map(|t| {
                            let cache_clone = cache.clone();
                            let paths_slice: Vec<Nibbles> = paths
                                [t * (10_000 / num_threads)..(t + 1) * (10_000 / num_threads)]
                                .to_vec();
                            thread::spawn(move || {
                                for path in &paths_slice {
                                    black_box(cache_clone.check_path(path));
                                }
                            })
                        })
                        .collect();

                    for handle in handles {
                        handle.join().expect("Thread panicked");
                    }
                })
            },
        );
    }

    group.finish();
}

// ===== PATH DEPTH BENCHMARKS =====

fn bench_varying_path_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("path_depth");

    for depth in [8, 16, 32, 64].iter() {
        let paths = generate_test_paths(1_000, *depth);
        let cache = HealingCache::new();
        cache.mark_exists_batch(&paths);

        group.bench_with_input(BenchmarkId::new("check", depth), depth, |b, _| {
            let mut idx = 0;
            b.iter(|| {
                let status = cache.check_path(&paths[idx % paths.len()]);
                idx += 1;
                black_box(status)
            })
        });
    }

    group.finish();
}

// ===== FALSE POSITIVE BENCHMARKS =====

fn bench_false_positive_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("false_positive");

    let cache = HealingCache::new();
    let known_paths = generate_test_paths(100_000, 32);
    let unknown_paths = generate_unknown_paths(10_000, 32);

    // Populate cache with known paths
    cache.mark_exists_batch(&known_paths);

    // Benchmark checking known paths (should be fast - cache hits)
    group.bench_function("known_paths", |b| {
        let mut idx = 0;
        b.iter(|| {
            let status = cache.check_path(&known_paths[idx % known_paths.len()]);
            idx += 1;
            black_box(status)
        })
    });

    // Benchmark checking unknown paths (filter returns DefinitelyMissing or false positive)
    group.bench_function("unknown_paths", |b| {
        let mut idx = 0;
        b.iter(|| {
            let status = cache.check_path(&unknown_paths[idx % unknown_paths.len()]);
            idx += 1;
            black_box(status)
        })
    });

    group.finish();
}

// ===== MEMORY PRESSURE BENCHMARKS =====

fn bench_large_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_cache");
    group.sample_size(50); // Reduce sample size for large benchmarks

    // Test with 500k paths (simulating real-world healing)
    let paths = generate_test_paths(500_000, 32);

    group.throughput(Throughput::Elements(500_000));

    group.bench_function("populate_500k", |b| {
        b.iter(|| {
            let cache = HealingCache::new();
            for chunk in paths.chunks(10_000) {
                cache.mark_exists_batch(chunk);
            }
            black_box(cache.stats())
        })
    });

    // Pre-populated cache lookup
    let cache = HealingCache::new();
    for chunk in paths.chunks(10_000) {
        cache.mark_exists_batch(chunk);
    }

    group.bench_function("lookup_500k", |b| {
        b.iter(|| {
            let mut found = 0u64;
            for path in paths.iter().take(10_000) {
                if matches!(
                    cache.check_path(path),
                    PathStatus::ConfirmedExists | PathStatus::ProbablyExists
                ) {
                    found += 1;
                }
            }
            black_box(found)
        })
    });

    group.finish();
}

// ===== SEQUENTIAL VS BATCH COMPARISON =====

fn bench_sequential_vs_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("seq_vs_batch");

    let cache = HealingCache::new();
    let paths = generate_test_paths(10_000, 32);
    cache.mark_exists_batch(&paths[..5000]);

    group.throughput(Throughput::Elements(10_000));

    // Sequential
    group.bench_function("sequential", |b| {
        b.iter(|| {
            let mut results = Vec::with_capacity(paths.len());
            for path in &paths {
                results.push(cache.check_path(path));
            }
            black_box(results)
        })
    });

    // Batch
    group.bench_function("batch", |b| {
        b.iter(|| black_box(cache.check_paths_batch(&paths)))
    });

    group.finish();
}

// ===== CRITERION GROUPS =====

criterion_group!(
    single_ops,
    bench_cache_check_single,
    bench_cache_mark_single,
);

criterion_group!(batch_ops, bench_cache_batch_operations,);

criterion_group!(
    cache_state,
    bench_cache_empty_vs_warm,
    bench_varying_path_depth,
);

criterion_group!(concurrent, bench_cache_concurrent,);

criterion_group!(
    comparison,
    bench_false_positive_overhead,
    bench_sequential_vs_batch,
);

criterion_group!(
    name = stress;
    config = Criterion::default().sample_size(30);
    targets = bench_large_cache
);

criterion_main!(single_ops, batch_ops, cache_state, concurrent, comparison, stress);
