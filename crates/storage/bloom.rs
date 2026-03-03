use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use ethrex_common::{Address, H256};
use fastbloom::AtomicBloomFilter;
use rustc_hash::FxBuildHasher;

const FALSE_POSITIVE_RATE: f64 = 0.01;

/// Bloom filter that tracks which (address, storage_key) pairs have non-zero
/// storage values. Used to skip expensive trie lookups for slots that were
/// never written to.
///
/// The filter is allocated lazily on first `insert()` to avoid ~240MB of
/// upfront memory when the bloom is never used (e.g., dev mode, testnets).
pub struct StorageBloomFilter {
    filter: OnceLock<AtomicBloomFilter<FxBuildHasher>>,
    capacity: usize,
    enabled: AtomicBool,
}

impl fmt::Debug for StorageBloomFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageBloomFilter").finish()
    }
}

impl StorageBloomFilter {
    pub fn new(capacity: usize) -> Self {
        Self {
            filter: OnceLock::new(),
            capacity,
            enabled: AtomicBool::new(false),
        }
    }

    /// Activate the bloom filter after it has been populated.
    /// Before this is called, `might_contain` always returns `true` (pass-through).
    ///
    /// # Precondition
    ///
    /// The filter MUST have been fully populated (via `insert`) for ALL
    /// (address, storage_key) pairs that exist in the trie before this is
    /// called. This includes genesis slots, snap-synced data, and all slots
    /// written during block processing. Calling `enable()` prematurely will
    /// cause false negatives that silently corrupt storage reads.
    #[allow(dead_code)]
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    /// Record that a non-zero value exists at (address, key).
    ///
    /// Called unconditionally on every non-zero storage write, even while the
    /// filter is disabled. This is intentional warm-up: the filter is populated
    /// in the background so it is ready when `enable()` is eventually called.
    pub fn insert(&self, address: Address, key: H256) {
        let bloom_key = Self::make_key(address, key);
        self.filter().insert(&bloom_key);
    }

    /// Returns `true` if the slot *might* contain a non-zero value.
    /// Returns `false` if the slot was definitely never written.
    /// When the filter is not yet enabled, always returns `true` (pass-through).
    pub fn might_contain(&self, address: Address, key: H256) -> bool {
        if !self.enabled.load(Ordering::Acquire) {
            return true;
        }
        let bloom_key = Self::make_key(address, key);
        self.filter().contains(&bloom_key)
    }

    fn filter(&self) -> &AtomicBloomFilter<FxBuildHasher> {
        self.filter.get_or_init(|| {
            AtomicBloomFilter::with_false_pos(FALSE_POSITIVE_RATE)
                .hasher(FxBuildHasher)
                .expected_items(self.capacity)
        })
    }

    fn make_key(address: Address, key: H256) -> [u8; 52] {
        let mut buf = [0u8; 52];
        buf[..20].copy_from_slice(address.as_bytes());
        buf[20..].copy_from_slice(key.as_bytes());
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> Address {
        Address::from([n; 20])
    }

    fn key(n: u8) -> H256 {
        H256::from([n; 32])
    }

    #[test]
    fn disabled_is_pass_through() {
        let bloom = StorageBloomFilter::new(1000);
        // Before enable, might_contain always returns true
        assert!(bloom.might_contain(addr(1), key(1)));
        assert!(bloom.might_contain(addr(99), key(255)));
    }

    #[test]
    fn no_false_negatives_after_enable() {
        let bloom = StorageBloomFilter::new(1000);
        bloom.insert(addr(1), key(10));
        bloom.insert(addr(2), key(20));
        bloom.enable();
        // Inserted keys must always return true
        assert!(bloom.might_contain(addr(1), key(10)));
        assert!(bloom.might_contain(addr(2), key(20)));
    }

    #[test]
    fn rejects_unknown_after_enable() {
        let bloom = StorageBloomFilter::new(1000);
        bloom.insert(addr(1), key(10));
        bloom.enable();
        // A never-inserted key should return false (with high probability)
        assert!(!bloom.might_contain(addr(99), key(99)));
    }

    #[test]
    fn make_key_distinctness() {
        // Different (address, key) pairs must produce different bloom keys
        let k1 = StorageBloomFilter::make_key(addr(1), key(2));
        let k2 = StorageBloomFilter::make_key(addr(2), key(1));
        assert_ne!(k1, k2);
    }
}
