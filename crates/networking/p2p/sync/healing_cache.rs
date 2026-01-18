//! Simple caching layer for snap sync healing
//!
//! This module provides a simple HashSet-based cache for tracking which trie paths
//! exist in the database, reducing expensive DB lookups during the healing phase.
//!
//! Simplified design: Single HashSet with a single lock for straightforward lookups.

use ethrex_trie::Nibbles;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::Arc;

/// Simple cache for tracking paths known to exist in the database during healing.
///
/// Uses a single HashSet for straightforward "exists or not" tracking.
#[derive(Debug)]
pub struct HealingCache {
    /// Set of paths confirmed to exist in DB
    existing_paths: RwLock<HashSet<Vec<u8>>>,
}

/// Statistics for cache performance monitoring (simplified)
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    /// Number of paths added to the cache
    pub paths_added: u64,
}

impl HealingCache {
    /// Create a new healing cache
    pub fn new() -> Self {
        Self {
            existing_paths: RwLock::new(HashSet::new()),
        }
    }

    /// Create a new healing cache with specified capacities (ignored, kept for API compatibility)
    pub fn with_capacity(_lru_capacity: usize, _filter_capacity: u64) -> Self {
        Self::new()
    }

    /// Check if a path exists in the cache.
    ///
    /// Returns:
    /// - `PathStatus::DefinitelyMissing` - Path is not in cache
    /// - `PathStatus::ConfirmedExists` - Path is in cache
    pub fn check_path(&self, path: &Nibbles) -> PathStatus {
        let key = path.as_ref();
        let paths = self.existing_paths.read();
        if paths.contains(key) {
            PathStatus::ConfirmedExists
        } else {
            PathStatus::DefinitelyMissing
        }
    }

    /// Check multiple paths at once, returning their statuses.
    pub fn check_paths_batch(&self, paths: &[Nibbles]) -> Vec<PathStatus> {
        let existing = self.existing_paths.read();
        paths
            .iter()
            .map(|path| {
                if existing.contains(path.as_ref()) {
                    PathStatus::ConfirmedExists
                } else {
                    PathStatus::DefinitelyMissing
                }
            })
            .collect()
    }

    /// Mark a path as confirmed to exist in the database.
    pub fn mark_exists(&self, path: &Nibbles) {
        let key = path.as_ref().to_vec();
        self.existing_paths.write().insert(key);
    }

    /// Mark multiple paths as confirmed to exist.
    pub fn mark_exists_batch(&self, paths: &[Nibbles]) {
        let mut existing = self.existing_paths.write();
        for path in paths {
            existing.insert(path.as_ref().to_vec());
        }
    }

    /// Get current cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            paths_added: self.existing_paths.read().len() as u64,
        }
    }

    /// Reset cache statistics (no-op for simplified version)
    pub fn reset_stats(&self) {
        // No-op - stats are derived from HashSet size
    }

    /// Clear all cached data
    pub fn clear(&self) {
        self.existing_paths.write().clear();
    }

    /// Get the number of cached paths
    pub fn len(&self) -> usize {
        self.existing_paths.read().len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.existing_paths.read().is_empty()
    }

    /// Get the current fill ratio (always returns 0.0 for simplified version)
    pub fn lru_fill_ratio(&self) -> f64 {
        0.0
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
    /// Path is not in the cache
    DefinitelyMissing,
    /// Path probably exists (kept for API compatibility, not used in simplified version)
    ProbablyExists,
    /// Path is confirmed to exist in the cache
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
    _lru_capacity: usize,
    _filter_capacity: u64,
) -> SharedHealingCache {
    Arc::new(HealingCache::new())
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

        // Last 50 should be missing
        for status in &statuses[50..100] {
            assert_eq!(*status, PathStatus::DefinitelyMissing);
        }
    }

    #[test]
    fn test_cache_stats() {
        let cache = HealingCache::new();

        let path = Nibbles::from_hex(vec![1, 2, 3]);

        // Initial stats should be zero
        let stats = cache.stats();
        assert_eq!(stats.paths_added, 0);

        // Mark path
        cache.mark_exists(&path);
        let stats = cache.stats();
        assert_eq!(stats.paths_added, 1);
    }

    #[test]
    fn test_cache_clear() {
        let cache = HealingCache::new();

        let path = Nibbles::from_hex(vec![1, 2, 3]);
        cache.mark_exists(&path);
        assert_eq!(cache.check_path(&path), PathStatus::ConfirmedExists);

        cache.clear();
        assert_eq!(cache.check_path(&path), PathStatus::DefinitelyMissing);
    }
}
