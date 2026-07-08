//! Read-through value cache for committed on-disk state.
//!
//! # What it is
//!
//! A bounded, sharded LRU that memoizes the result of a trie/flat-KV point
//! lookup against the on-disk state (the `*_trie_nodes` / `*_flatkeyvalue`
//! column families). It sits *beneath* the in-memory diff-layer overlay
//! ([`crate::layering::TrieLayerCache`]) in the read cascade:
//!
//! ```text
//! TrieWrapper::get
//!   → TrieLayerCache overlay   (last ~128 blocks of writes)
//!   → CleanCache               (this: committed on-disk values)
//!   → disk (RocksDB)           (fill CleanCache on miss)
//! ```
//!
//! # Why it is correct by construction
//!
//! The on-disk state is a single-version, path-keyed store, and
//! [`crate::backend`]'s `get` ignores the state root: it is a plain point get on
//! the prefixed path. So an entry here is just a memoization of that call and is
//! independent of which state root a trie was opened at. The overlay always
//! shadows any key written in the recent window, so a stale entry can never be
//! observed for a recently-written key.
//!
//! The only invalidation needed is at the points that mutate the on-disk state
//! CFs. In the synced regime (the only regime where this cache is active, see
//! the gate in `store.rs`) that is exactly two writers:
//! - `commit_trie_layers` folds an overlay layer to disk — it invalidates each
//!   committed key.
//! - `write_storage_trie_nodes_batch` (bulk healing path) — it clears the cache.
//!
//! # Memory
//!
//! Memory is hard-bounded by bytes, not entry count. The budget is split evenly
//! across [`SHARD_COUNT`] shards; each shard evicts its LRU tail until it is
//! under its share. The accounted weight of an entry is
//! `key.len() + value.len() + ENTRY_OVERHEAD_BYTES`, so the reported figure
//! tracks real resident memory rather than just payload. See
//! [`CleanCache::used_bytes`] and [`CleanCache::max_bytes`].

use lru::LruCache;
use rustc_hash::FxBuildHasher;
use std::fmt;
use std::sync::Mutex;

/// Fixed per-entry bookkeeping overhead added to key+value bytes: the LRU node,
/// the two boxed slices, and the hash-map slot. Approximate on purpose; it only
/// needs to keep the byte bound from undercounting real memory.
const ENTRY_OVERHEAD_BYTES: usize = 64;

/// Number of independently-locked shards. Reads take a per-shard lock, so more
/// shards means less contention when the executor and prefetch workers hit the
/// cache in parallel. Must be a power of two (used as a mask).
const SHARD_COUNT: usize = 64;

type Key = Box<[u8]>;
type Val = Box<[u8]>;

struct Shard {
    map: LruCache<Key, Val, FxBuildHasher>,
    used_bytes: usize,
    max_bytes: usize,
}

impl Shard {
    fn new(max_bytes: usize) -> Self {
        Self {
            map: LruCache::unbounded_with_hasher(FxBuildHasher),
            used_bytes: 0,
            max_bytes,
        }
    }

    fn weight(key_len: usize, val_len: usize) -> usize {
        key_len + val_len + ENTRY_OVERHEAD_BYTES
    }

    fn get(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        self.map.get(key).map(|v| v.to_vec())
    }

    fn insert(&mut self, key: Key, val: &[u8]) {
        let key_len = key.len();
        let weight = Self::weight(key_len, val.len());
        // Never cache a single entry larger than the whole shard budget: it
        // would evict everything and then itself on the next insert.
        if weight > self.max_bytes {
            return;
        }
        if let Some(old) = self.map.put(key, val.into()) {
            // Same key, so the old entry's key length equals `key_len`.
            let old_weight = Self::weight(key_len, old.len());
            self.used_bytes = self.used_bytes.saturating_sub(old_weight);
        }
        self.used_bytes += weight;
        while self.used_bytes > self.max_bytes {
            match self.map.pop_lru() {
                Some((k, v)) => {
                    let w = Self::weight(k.len(), v.len());
                    self.used_bytes = self.used_bytes.saturating_sub(w);
                }
                None => break,
            }
        }
    }

    fn invalidate(&mut self, key: &[u8]) {
        if let Some(old) = self.map.pop(key) {
            let w = Self::weight(key.len(), old.len());
            self.used_bytes = self.used_bytes.saturating_sub(w);
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.used_bytes = 0;
    }
}

/// Bounded, sharded, read-through cache of committed on-disk state values.
///
/// Cloneable-by-`Arc` in [`crate::store::Store`]; every trie opened on the
/// synced read path shares one instance.
pub struct CleanCache {
    shards: Box<[Mutex<Shard>]>,
    /// Total byte budget across all shards (sum of per-shard `max_bytes`).
    max_bytes: usize,
}

impl fmt::Debug for CleanCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CleanCache")
            .field("shards", &self.shards.len())
            .field("max_bytes", &self.max_bytes)
            .field("used_bytes", &self.used_bytes())
            .field("len", &self.len())
            .finish()
    }
}

impl CleanCache {
    /// Creates a cache with the given total byte budget, split evenly across
    /// [`SHARD_COUNT`] shards. A budget of `0` yields a cache whose shards can
    /// hold nothing (every insert is a no-op); prefer gating with `None` at the
    /// call site instead of relying on this.
    pub fn new(total_max_bytes: usize) -> Self {
        let per_shard = total_max_bytes / SHARD_COUNT;
        let shards = (0..SHARD_COUNT)
            .map(|_| Mutex::new(Shard::new(per_shard)))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            shards,
            max_bytes: per_shard * SHARD_COUNT,
        }
    }

    /// Selects a shard from the leading bytes of the key. Keys are keccak-derived
    /// nibble paths, so the leading nibbles are uniformly distributed.
    fn shard(&self, key: &[u8]) -> &Mutex<Shard> {
        let a = key.first().copied().unwrap_or(0) as usize;
        let b = key.get(1).copied().unwrap_or(0) as usize;
        let idx = ((a << 4) | b) & (self.shards.len() - 1);
        &self.shards[idx]
    }

    /// Returns the cached value for `key`, promoting it to most-recently-used.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.shard(key).lock().ok()?.get(key)
    }

    /// Inserts (or refreshes) `key -> value`, evicting the shard's LRU tail if
    /// the shard is over budget. `key` is consumed to avoid a second allocation
    /// (the caller already built the prefixed path).
    pub fn insert(&self, key: Key, value: &[u8]) {
        if let Ok(mut shard) = self.shard(&key).lock() {
            shard.insert(key, value);
        }
    }

    /// Removes `key`. Called for every key mutated on disk by `commit_trie_layers`.
    pub fn invalidate(&self, key: &[u8]) {
        if let Ok(mut shard) = self.shard(key).lock() {
            shard.invalidate(key);
        }
    }

    /// Drops every entry. Used by bulk on-disk mutations that bypass the normal
    /// commit path (`write_storage_trie_nodes_batch`).
    pub fn clear(&self) {
        for shard in self.shards.iter() {
            if let Ok(mut shard) = shard.lock() {
                shard.clear();
            }
        }
    }

    /// Total byte budget (upper bound on resident memory for cached entries).
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Currently accounted resident bytes across all shards.
    pub fn used_bytes(&self) -> usize {
        self.shards
            .iter()
            .filter_map(|s| s.lock().ok().map(|s| s.used_bytes))
            .sum()
    }

    /// Number of cached entries across all shards.
    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .filter_map(|s| s.lock().ok().map(|s| s.map.len()))
            .sum()
    }

    /// Returns true if no entries are cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> Vec<u8> {
        // Distinct across shards: vary the leading bytes.
        vec![n, n.wrapping_mul(7), n.wrapping_add(3), 0xaa]
    }

    #[test]
    fn get_after_insert_hits() {
        let c = CleanCache::new(1 << 20);
        c.insert(key(1).into_boxed_slice(), b"value-1");
        assert_eq!(c.get(&key(1)), Some(b"value-1".to_vec()));
        assert_eq!(c.get(&key(2)), None);
    }

    #[test]
    fn invalidate_removes_entry() {
        let c = CleanCache::new(1 << 20);
        c.insert(key(1).into_boxed_slice(), b"v");
        c.invalidate(&key(1));
        assert_eq!(c.get(&key(1)), None);
        assert!(c.is_empty());
    }

    #[test]
    fn clear_drops_all() {
        let c = CleanCache::new(1 << 20);
        for i in 0..50u8 {
            c.insert(key(i).into_boxed_slice(), b"v");
        }
        assert!(!c.is_empty());
        c.clear();
        assert!(c.is_empty());
        assert_eq!(c.used_bytes(), 0);
    }

    #[test]
    fn memory_is_bounded_by_budget() {
        // Small budget; hammer one shard with many entries and assert the
        // per-shard byte bound is never exceeded and old entries are evicted.
        let total = SHARD_COUNT * 4096; // 4 KiB per shard
        let c = CleanCache::new(total);
        // All these keys land in the same shard (identical leading two bytes).
        for i in 0..10_000u32 {
            let mut k = vec![0u8, 0u8];
            k.extend_from_slice(&i.to_le_bytes());
            c.insert(k.into_boxed_slice(), &[7u8; 256]);
        }
        assert!(
            c.used_bytes() <= c.max_bytes(),
            "used {} must not exceed budget {}",
            c.used_bytes(),
            c.max_bytes()
        );
        // The most recently inserted key must still be present (LRU keeps the tail).
        let mut last = vec![0u8, 0u8];
        last.extend_from_slice(&9_999u32.to_le_bytes());
        assert_eq!(c.get(&last), Some(vec![7u8; 256]));
    }

    #[test]
    fn oversized_entry_is_not_cached() {
        let c = CleanCache::new(SHARD_COUNT * 128); // 128 B per shard
        c.insert(key(1).into_boxed_slice(), &[0u8; 4096]);
        assert_eq!(c.get(&key(1)), None);
        assert_eq!(c.used_bytes(), 0);
    }
}
