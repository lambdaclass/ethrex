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
