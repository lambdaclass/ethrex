use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use ethrex_common::{Address, H256};
use fastbloom::AtomicBloomFilter;
use rustc_hash::FxBuildHasher;

const FALSE_POSITIVE_RATE: f64 = 0.01;

/// Bloom filter that tracks which (address, storage_key) pairs have non-zero
/// storage values. Used to skip expensive trie lookups for slots that were
/// never written to.
pub struct StorageBloomFilter {
    filter: AtomicBloomFilter<FxBuildHasher>,
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
            filter: AtomicBloomFilter::with_false_pos(FALSE_POSITIVE_RATE)
                .hasher(FxBuildHasher)
                .expected_items(capacity),
            enabled: AtomicBool::new(false),
        }
    }

    /// Activate the bloom filter after it has been populated.
    /// Before this is called, `might_contain` always returns `true` (pass-through).
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    /// Record that a non-zero value exists at (address, key).
    pub fn insert(&self, address: Address, key: H256) {
        let bloom_key = Self::make_key(address, key);
        self.filter.insert(&bloom_key);
    }

    /// Returns `true` if the slot *might* contain a non-zero value.
    /// Returns `false` if the slot was definitely never written.
    /// When the filter is not yet enabled, always returns `true` (pass-through).
    pub fn might_contain(&self, address: Address, key: H256) -> bool {
        if !self.enabled.load(Ordering::Acquire) {
            return true;
        }
        let bloom_key = Self::make_key(address, key);
        self.filter.contains(&bloom_key)
    }

    fn make_key(address: Address, key: H256) -> [u8; 52] {
        let mut buf = [0u8; 52];
        buf[..20].copy_from_slice(address.as_bytes());
        buf[20..].copy_from_slice(key.as_bytes());
        buf
    }
}
