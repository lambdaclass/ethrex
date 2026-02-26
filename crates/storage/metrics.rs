use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight atomic counters for storage layer instrumentation.
///
/// These counters have negligible overhead (single atomic increment per operation)
/// and are used to measure storage access patterns for optimization decisions.
pub static STORAGE_METRICS: StorageMetrics = StorageMetrics::new();

pub struct StorageMetrics {
    /// Number of reads served from flat key-value CFs
    pub flat_hits: AtomicU64,
    /// Number of reads served from trie node CFs
    pub trie_node_reads: AtomicU64,
    /// Number of trie layer cache hits (found in in-memory layers)
    pub layer_cache_hits: AtomicU64,
    /// Number of trie layer cache misses (fell through to RocksDB)
    pub layer_cache_misses: AtomicU64,
    /// Number of bloom filter checks (total)
    pub bloom_checks: AtomicU64,
    /// Number of bloom filter false positives (bloom said yes, layers said no)
    pub bloom_false_positives: AtomicU64,
    /// Cumulative snapshot layer depth traversed during lookups
    pub layer_depth_total: AtomicU64,
    /// Number of layer cache lookups that traversed at least one layer
    pub layer_depth_count: AtomicU64,
}

impl StorageMetrics {
    const fn new() -> Self {
        Self {
            flat_hits: AtomicU64::new(0),
            trie_node_reads: AtomicU64::new(0),
            layer_cache_hits: AtomicU64::new(0),
            layer_cache_misses: AtomicU64::new(0),
            bloom_checks: AtomicU64::new(0),
            bloom_false_positives: AtomicU64::new(0),
            layer_depth_total: AtomicU64::new(0),
            layer_depth_count: AtomicU64::new(0),
        }
    }

    /// Returns a snapshot of all counters for logging/reporting.
    pub fn snapshot(&self) -> StorageMetricsSnapshot {
        StorageMetricsSnapshot {
            flat_hits: self.flat_hits.load(Ordering::Relaxed),
            trie_node_reads: self.trie_node_reads.load(Ordering::Relaxed),
            layer_cache_hits: self.layer_cache_hits.load(Ordering::Relaxed),
            layer_cache_misses: self.layer_cache_misses.load(Ordering::Relaxed),
            bloom_checks: self.bloom_checks.load(Ordering::Relaxed),
            bloom_false_positives: self.bloom_false_positives.load(Ordering::Relaxed),
            layer_depth_total: self.layer_depth_total.load(Ordering::Relaxed),
            layer_depth_count: self.layer_depth_count.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StorageMetricsSnapshot {
    pub flat_hits: u64,
    pub trie_node_reads: u64,
    pub layer_cache_hits: u64,
    pub layer_cache_misses: u64,
    pub bloom_checks: u64,
    pub bloom_false_positives: u64,
    pub layer_depth_total: u64,
    pub layer_depth_count: u64,
}

impl StorageMetricsSnapshot {
    /// Bloom false positive rate as a percentage (0.0 - 100.0)
    pub fn bloom_fp_rate(&self) -> f64 {
        if self.bloom_checks == 0 {
            0.0
        } else {
            (self.bloom_false_positives as f64 / self.bloom_checks as f64) * 100.0
        }
    }

    /// Average layer depth per lookup
    pub fn avg_layer_depth(&self) -> f64 {
        if self.layer_depth_count == 0 {
            0.0
        } else {
            self.layer_depth_total as f64 / self.layer_depth_count as f64
        }
    }
}

impl std::fmt::Display for StorageMetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "flat_hits={} trie_reads={} cache_hits={} cache_misses={} bloom_checks={} bloom_fp={} ({:.2}%) avg_layer_depth={:.1}",
            self.flat_hits,
            self.trie_node_reads,
            self.layer_cache_hits,
            self.layer_cache_misses,
            self.bloom_checks,
            self.bloom_false_positives,
            self.bloom_fp_rate(),
            self.avg_layer_depth(),
        )
    }
}
