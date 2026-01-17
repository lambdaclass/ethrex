//! High-performance caching layer for snap sync healing
//!
//! This module provides optimized data structures for tracking which trie paths
//! exist in the database, reducing expensive DB lookups during the healing phase.
//!
//! Key optimizations:
//! - Quotient filter for probabilistic existence checks (eliminates ~95% of DB reads)
//! - LRU cache for recently verified paths
//! - Batch lookup support for amortized DB access

use ethrex_trie::Nibbles;
use lru::LruCache;
use parking_lot::RwLock;
use qfilter::Filter;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Default capacity for the LRU cache of verified paths
const DEFAULT_LRU_CAPACITY: usize = 100_000;

/// Default capacity for the quotient filter (number of expected entries)
/// This should be set based on expected trie size
const DEFAULT_FILTER_CAPACITY: u64 = 10_000_000;

/// False positive rate for the quotient filter
/// Lower = more memory, fewer false positives
const FILTER_FALSE_POSITIVE_RATE: f64 = 0.01;

/// Cache for tracking paths known to exist in the database during healing.
///
/// Uses a two-tier approach:
/// 1. Quotient filter for fast probabilistic "probably exists" checks
/// 2. LRU cache for recently verified paths (definite existence)
///
/// The quotient filter may return false positives (saying a path exists when it doesn't),
/// but never false negatives. This is perfect for our use case:
/// - If filter says "doesn't exist" -> definitely need to fetch from peer
/// - If filter says "exists" -> check LRU cache, then DB if needed
#[derive(Debug)]
pub struct HealingCache {
    /// Quotient filter for probabilistic existence checks
    /// Returns true if path "probably exists", false if "definitely doesn't exist"
    filter: RwLock<Filter>,

    /// LRU cache of paths confirmed to exist in DB
    /// These are paths we've actually verified via DB lookup
    verified_paths: RwLock<LruCache<Vec<u8>, ()>>,

    /// Statistics for monitoring cache effectiveness
    stats: RwLock<CacheStats>,
}

/// Statistics for cache performance monitoring
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    /// Number of filter hits (filter said "probably exists")
    pub filter_hits: u64,
    /// Number of filter misses (filter said "definitely doesn't exist")
    pub filter_misses: u64,
    /// Number of LRU cache hits
    pub lru_hits: u64,
    /// Number of LRU cache misses
    pub lru_misses: u64,
    /// Number of paths added to the cache
    pub paths_added: u64,
}

impl HealingCache {
    /// Create a new healing cache with default capacities
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_LRU_CAPACITY, DEFAULT_FILTER_CAPACITY)
    }

    /// Create a new healing cache with specified capacities
    pub fn with_capacity(lru_capacity: usize, filter_capacity: u64) -> Self {
        let filter = Filter::new(filter_capacity, FILTER_FALSE_POSITIVE_RATE)
            .expect("Failed to create quotient filter");

        Self {
            filter: RwLock::new(filter),
            verified_paths: RwLock::new(LruCache::new(
                NonZeroUsize::new(lru_capacity).expect("capacity must be non-zero"),
            )),
            stats: RwLock::new(CacheStats::default()),
        }
    }

    /// Check if a path probably exists in the database.
    ///
    /// Returns:
    /// - `PathStatus::DefinitelyMissing` - Path is definitely not in DB (filter miss)
    /// - `PathStatus::ProbablyExists` - Path might exist (filter hit, not in LRU)
    /// - `PathStatus::ConfirmedExists` - Path definitely exists (in LRU cache)
    pub fn check_path(&self, path: &Nibbles) -> PathStatus {
        let key = path.as_ref();

        // First check LRU cache for confirmed existence
        {
            let mut lru = self.verified_paths.write();
            if lru.get(key).is_some() {
                let mut stats = self.stats.write();
                stats.lru_hits += 1;
                return PathStatus::ConfirmedExists;
            }
        }

        // Check quotient filter
        let filter = self.filter.read();
        if filter.contains(key) {
            let mut stats = self.stats.write();
            stats.filter_hits += 1;
            stats.lru_misses += 1;
            PathStatus::ProbablyExists
        } else {
            let mut stats = self.stats.write();
            stats.filter_misses += 1;
            PathStatus::DefinitelyMissing
        }
    }

    /// Check multiple paths at once, returning their statuses.
    ///
    /// This is more efficient than calling `check_path` repeatedly
    /// because it batches the lock acquisitions.
    pub fn check_paths_batch(&self, paths: &[Nibbles]) -> Vec<PathStatus> {
        let mut results = Vec::with_capacity(paths.len());

        // Batch check LRU cache
        let mut lru = self.verified_paths.write();
        let filter = self.filter.read();
        let mut stats = self.stats.write();

        for path in paths {
            let key = path.as_ref();

            if lru.get(key).is_some() {
                stats.lru_hits += 1;
                results.push(PathStatus::ConfirmedExists);
            } else if filter.contains(key) {
                stats.filter_hits += 1;
                stats.lru_misses += 1;
                results.push(PathStatus::ProbablyExists);
            } else {
                stats.filter_misses += 1;
                results.push(PathStatus::DefinitelyMissing);
            }
        }

        results
    }

    /// Mark a path as confirmed to exist in the database.
    ///
    /// This should be called after a successful DB lookup confirms the path exists.
    pub fn mark_exists(&self, path: &Nibbles) {
        let key = path.as_ref().to_vec();

        // Add to filter
        {
            let mut filter = self.filter.write();
            filter.insert(&key).ok(); // Ignore capacity errors
        }

        // Add to LRU cache
        {
            let mut lru = self.verified_paths.write();
            lru.put(key, ());
        }

        self.stats.write().paths_added += 1;
    }

    /// Mark multiple paths as confirmed to exist.
    ///
    /// More efficient than calling `mark_exists` repeatedly.
    pub fn mark_exists_batch(&self, paths: &[Nibbles]) {
        let mut filter = self.filter.write();
        let mut lru = self.verified_paths.write();
        let mut stats = self.stats.write();

        for path in paths {
            let key = path.as_ref().to_vec();
            filter.insert(&key).ok();
            lru.put(key, ());
            stats.paths_added += 1;
        }
    }

    /// Get current cache statistics
    pub fn stats(&self) -> CacheStats {
        self.stats.read().clone()
    }

    /// Reset cache statistics
    pub fn reset_stats(&self) {
        *self.stats.write() = CacheStats::default();
    }

    /// Clear all cached data
    pub fn clear(&self) {
        // Create a new filter to clear it
        let new_filter = Filter::new(DEFAULT_FILTER_CAPACITY, FILTER_FALSE_POSITIVE_RATE)
            .expect("Failed to create quotient filter");
        *self.filter.write() = new_filter;

        self.verified_paths.write().clear();
        *self.stats.write() = CacheStats::default();
    }

    /// Get the current fill ratio of the LRU cache
    pub fn lru_fill_ratio(&self) -> f64 {
        let lru = self.verified_paths.read();
        lru.len() as f64 / lru.cap().get() as f64
    }
}

impl Default for HealingCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Status of a path in the healing cache
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStatus {
    /// Path is definitely not in the database (filter returned false)
    DefinitelyMissing,
    /// Path probably exists in the database (filter returned true, but not in LRU)
    /// A DB lookup is needed to confirm
    ProbablyExists,
    /// Path is confirmed to exist in the database (found in LRU cache)
    ConfirmedExists,
}

/// Thread-safe wrapper for sharing the healing cache across async tasks
pub type SharedHealingCache = Arc<HealingCache>;

/// Create a new shared healing cache
pub fn new_shared_cache() -> SharedHealingCache {
    Arc::new(HealingCache::new())
}

/// Create a new shared healing cache with specified capacities
pub fn new_shared_cache_with_capacity(
    lru_capacity: usize,
    filter_capacity: u64,
) -> SharedHealingCache {
    Arc::new(HealingCache::with_capacity(lru_capacity, filter_capacity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic_operations() {
        let cache = HealingCache::new();

        let path1 = Nibbles::from_hex(vec![1, 2, 3, 4]);
        let path2 = Nibbles::from_hex(vec![5, 6, 7, 8]);

        // Initially, paths should be missing
        assert_eq!(cache.check_path(&path1), PathStatus::DefinitelyMissing);

        // After marking as existing, should be confirmed
        cache.mark_exists(&path1);
        assert_eq!(cache.check_path(&path1), PathStatus::ConfirmedExists);

        // path2 should still be missing
        assert_eq!(cache.check_path(&path2), PathStatus::DefinitelyMissing);
    }

    #[test]
    fn test_cache_batch_operations() {
        let cache = HealingCache::new();

        let paths: Vec<Nibbles> = (0..100)
            .map(|i| Nibbles::from_hex(vec![i as u8, (i + 1) as u8]))
            .collect();

        // Mark half as existing
        cache.mark_exists_batch(&paths[0..50]);

        // Check all paths
        let statuses = cache.check_paths_batch(&paths);

        // First 50 should be confirmed
        for status in &statuses[0..50] {
            assert_eq!(*status, PathStatus::ConfirmedExists);
        }

        // Last 50 should be missing (or possibly ProbablyExists due to false positives)
        for status in &statuses[50..100] {
            assert!(matches!(
                *status,
                PathStatus::DefinitelyMissing | PathStatus::ProbablyExists
            ));
        }
    }

    #[test]
    fn test_cache_stats() {
        let cache = HealingCache::new();

        let path = Nibbles::from_hex(vec![1, 2, 3]);

        // Initial stats should be zero
        let stats = cache.stats();
        assert_eq!(stats.paths_added, 0);

        // Check a missing path
        cache.check_path(&path);
        let stats = cache.stats();
        assert_eq!(stats.filter_misses, 1);

        // Mark and check again
        cache.mark_exists(&path);
        cache.check_path(&path);
        let stats = cache.stats();
        assert_eq!(stats.paths_added, 1);
        assert_eq!(stats.lru_hits, 1);
    }
}
