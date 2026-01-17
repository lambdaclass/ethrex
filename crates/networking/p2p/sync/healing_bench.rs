//! Performance benchmarks for healing optimizations
//!
//! These benchmarks compare the original sequential child checks
//! vs the optimized batch + cache approach.

#[cfg(test)]
mod benchmarks {
    use super::super::healing_cache::{HealingCache, PathStatus};
    use ethrex_trie::Nibbles;
    use std::time::Instant;

    /// Generates random nibble paths for benchmarking
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

    #[test]
    fn bench_cache_check_performance() {
        let cache = HealingCache::new();
        let paths = generate_test_paths(100_000, 32);

        // Warm up: add half the paths to the cache
        let half = paths.len() / 2;
        cache.mark_exists_batch(&paths[..half]);

        // Benchmark: check all paths
        let start = Instant::now();
        let mut confirmed = 0;
        let mut probably = 0;
        let mut missing = 0;

        for path in &paths {
            match cache.check_path(path) {
                PathStatus::ConfirmedExists => confirmed += 1,
                PathStatus::ProbablyExists => probably += 1,
                PathStatus::DefinitelyMissing => missing += 1,
            }
        }

        let elapsed = start.elapsed();
        let ops_per_sec = paths.len() as f64 / elapsed.as_secs_f64();

        println!("\n=== Cache Check Performance ===");
        println!("Total paths checked: {}", paths.len());
        println!("  Confirmed exists: {}", confirmed);
        println!("  Probably exists: {}", probably);
        println!("  Definitely missing: {}", missing);
        println!("Time: {:?}", elapsed);
        println!("Operations/sec: {:.0}", ops_per_sec);
        println!();

        // Should be very fast - millions of ops/sec
        assert!(
            ops_per_sec > 100_000.0,
            "Cache check too slow: {} ops/sec",
            ops_per_sec
        );
    }

    #[test]
    fn bench_cache_batch_operations() {
        let cache = HealingCache::new();
        let paths = generate_test_paths(10_000, 32);

        // Benchmark batch add
        let start = Instant::now();
        cache.mark_exists_batch(&paths);
        let add_elapsed = start.elapsed();

        // Benchmark batch check
        let start = Instant::now();
        let statuses = cache.check_paths_batch(&paths);
        let check_elapsed = start.elapsed();

        let add_ops_per_sec = paths.len() as f64 / add_elapsed.as_secs_f64();
        let check_ops_per_sec = paths.len() as f64 / check_elapsed.as_secs_f64();

        println!("\n=== Batch Operations Performance ===");
        println!("Paths: {}", paths.len());
        println!("Batch add time: {:?} ({:.0} ops/sec)", add_elapsed, add_ops_per_sec);
        println!(
            "Batch check time: {:?} ({:.0} ops/sec)",
            check_elapsed, check_ops_per_sec
        );
        println!();

        // All should be confirmed after adding
        let confirmed_count = statuses
            .iter()
            .filter(|s| matches!(s, PathStatus::ConfirmedExists))
            .count();
        assert_eq!(confirmed_count, paths.len());
    }

    #[test]
    fn bench_cache_vs_sequential() {
        let cache = HealingCache::new();
        let paths = generate_test_paths(50_000, 32);

        // Pre-populate with half
        cache.mark_exists_batch(&paths[..paths.len() / 2]);

        // Sequential checks (simulating original approach without cache)
        let start = Instant::now();
        let mut seq_results = Vec::with_capacity(paths.len());
        for path in &paths {
            // Simulate a "DB lookup" with just a cache check
            // In real code this would be much slower
            seq_results.push(cache.check_path(path));
        }
        let sequential_elapsed = start.elapsed();

        // Batch checks (optimized approach)
        let start = Instant::now();
        let _batch_results = cache.check_paths_batch(&paths);
        let batch_elapsed = start.elapsed();

        let speedup = sequential_elapsed.as_secs_f64() / batch_elapsed.as_secs_f64();

        println!("\n=== Sequential vs Batch Performance ===");
        println!("Paths: {}", paths.len());
        println!("Sequential time: {:?}", sequential_elapsed);
        println!("Batch time: {:?}", batch_elapsed);
        println!("Speedup: {:.2}x", speedup);
        println!();

        // Batch should be faster due to reduced lock contention
        assert!(
            speedup > 0.5,
            "Batch should not be significantly slower than sequential"
        );
    }

    #[test]
    fn bench_cache_memory_efficiency() {
        let cache = HealingCache::new();

        // Add many paths
        let paths = generate_test_paths(500_000, 32);

        let start = Instant::now();
        for chunk in paths.chunks(10_000) {
            cache.mark_exists_batch(chunk);
        }
        let elapsed = start.elapsed();

        let stats = cache.stats();
        let fill_ratio = cache.lru_fill_ratio();

        println!("\n=== Memory Efficiency ===");
        println!("Total paths added: {}", paths.len());
        println!("Paths tracked by stats: {}", stats.paths_added);
        println!("LRU fill ratio: {:.2}%", fill_ratio * 100.0);
        println!("Time to add all: {:?}", elapsed);
        println!();

        // Verify paths were added (stats tracks total, LRU is bounded)
        assert!(stats.paths_added > 0, "Should have added paths");
    }

    #[test]
    fn bench_false_positive_rate() {
        let cache = HealingCache::new();

        // Add a set of paths
        let known_paths = generate_test_paths(100_000, 32);
        cache.mark_exists_batch(&known_paths);

        // Check paths that were NOT added (generate different paths)
        let unknown_paths: Vec<Nibbles> = (0..50_000)
            .map(|i| {
                let bytes: Vec<u8> = (0..32)
                    .map(|j| ((i * 19 + j * 37 + 12345) % 256) as u8)
                    .collect();
                Nibbles::from_bytes(&bytes)
            })
            .collect();

        let mut false_positives = 0;
        let mut true_negatives = 0;

        for path in &unknown_paths {
            match cache.check_path(path) {
                PathStatus::ConfirmedExists => false_positives += 1, // Should not happen for unknown
                PathStatus::ProbablyExists => false_positives += 1,  // Filter false positive
                PathStatus::DefinitelyMissing => true_negatives += 1,
            }
        }

        let fp_rate = false_positives as f64 / unknown_paths.len() as f64;

        println!("\n=== False Positive Rate ===");
        println!("Unknown paths checked: {}", unknown_paths.len());
        println!("True negatives: {}", true_negatives);
        println!("False positives: {}", false_positives);
        println!("False positive rate: {:.2}%", fp_rate * 100.0);
        println!();

        // False positive rate should be close to configured rate (1%)
        // Allow some margin
        assert!(
            fp_rate < 0.05,
            "False positive rate too high: {:.2}%",
            fp_rate * 100.0
        );
    }
}
