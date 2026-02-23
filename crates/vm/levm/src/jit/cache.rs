//! JIT code cache.
//!
//! Stores compiled function pointers keyed by (bytecode hash, fork).
//! The cache is thread-safe and designed for concurrent read access
//! with infrequent writes (compilation events).

use ethrex_common::H256;
use ethrex_common::types::Fork;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

/// Cache key combining bytecode hash and fork.
///
/// The same bytecode compiled at different forks produces different native code
/// (opcodes, gas costs are baked in at compile time), so the cache must
/// distinguish them.
pub type CacheKey = (H256, Fork);

/// Metadata and function pointer for a JIT-compiled bytecode.
///
/// # Safety
///
/// The function pointer is obtained from the JIT compiler (revmc/LLVM)
/// and points to executable memory managed by the compiler's runtime.
/// The pointer remains valid as long as the compiler context that produced
/// it is alive. The `tokamak-jit` crate is responsible for ensuring this
/// lifetime invariant.
pub struct CompiledCode {
    /// Type-erased function pointer to the compiled code.
    /// The actual signature is `RawEvmCompilerFn` from revmc-context,
    /// but we erase it here to avoid depending on revmc in LEVM.
    ptr: *const (),
    /// Size of the original bytecode (for metrics).
    pub bytecode_size: usize,
    /// Number of basic blocks in the compiled code.
    pub basic_block_count: usize,
    /// LLVM function ID for memory management on eviction.
    /// None if the backend doesn't support function-level freeing.
    pub func_id: Option<u32>,
}

impl CompiledCode {
    /// Create a new `CompiledCode` from a raw function pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` points to valid, executable JIT-compiled
    /// code that conforms to the expected calling convention. The pointer must remain
    /// valid for the lifetime of this `CompiledCode` value.
    #[allow(unsafe_code)]
    pub unsafe fn new(
        ptr: *const (),
        bytecode_size: usize,
        basic_block_count: usize,
        func_id: Option<u32>,
    ) -> Self {
        Self {
            ptr,
            bytecode_size,
            basic_block_count,
            func_id,
        }
    }

    /// Get the raw function pointer.
    pub fn as_ptr(&self) -> *const () {
        self.ptr
    }
}

// SAFETY: The function pointer is produced by LLVM JIT and points to immutable,
// position-independent machine code. It is safe to share across threads as the
// compiled code is never mutated after creation.
#[expect(unsafe_code)]
unsafe impl Send for CompiledCode {}
#[expect(unsafe_code)]
unsafe impl Sync for CompiledCode {}

impl std::fmt::Debug for CompiledCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledCode")
            .field("ptr", &self.ptr)
            .field("bytecode_size", &self.bytecode_size)
            .field("basic_block_count", &self.basic_block_count)
            .field("func_id", &self.func_id)
            .finish()
    }
}

/// Inner state for the code cache (behind RwLock).
#[derive(Debug)]
struct CodeCacheInner {
    entries: HashMap<CacheKey, Arc<CompiledCode>>,
    insertion_order: VecDeque<CacheKey>,
    max_entries: usize,
}

/// Thread-safe cache of JIT-compiled bytecodes with FIFO eviction.
///
/// When the cache reaches `max_entries`, the oldest entry (by insertion time)
/// is evicted. `get()` does not update access order, so this is FIFO, not LRU.
/// Note: LLVM JIT memory is NOT freed on eviction (revmc limitation).
/// The eviction only prevents HashMap metadata growth.
#[derive(Debug, Clone)]
pub struct CodeCache {
    inner: Arc<RwLock<CodeCacheInner>>,
}

impl CodeCache {
    /// Create a new empty code cache with the given capacity.
    pub fn with_max_entries(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CodeCacheInner {
                entries: HashMap::new(),
                insertion_order: VecDeque::new(),
                max_entries,
            })),
        }
    }

    /// Create a new empty code cache with default capacity (1024).
    pub fn new() -> Self {
        Self::with_max_entries(1024)
    }

    /// Look up compiled code by (bytecode hash, fork).
    pub fn get(&self, key: &CacheKey) -> Option<Arc<CompiledCode>> {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let inner = self.inner.read().unwrap();
        inner.entries.get(key).cloned()
    }

    /// Insert compiled code into the cache, evicting the oldest entry if at capacity.
    ///
    /// Returns the evicted entry's `func_id` if an eviction occurred and the evicted
    /// entry had a function ID, so the caller can free the LLVM memory.
    pub fn insert(&self, key: CacheKey, code: CompiledCode) -> Option<u32> {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut inner = self.inner.write().unwrap();

        // If already present, just update the value (no eviction needed)
        if let std::collections::hash_map::Entry::Occupied(mut e) = inner.entries.entry(key) {
            e.insert(Arc::new(code));
            return None;
        }

        // Evict oldest if at capacity
        let mut evicted_func_id = None;
        if inner.max_entries > 0
            && inner.entries.len() >= inner.max_entries
            && let Some(oldest) = inner.insertion_order.pop_front()
            && let Some(evicted) = inner.entries.remove(&oldest)
        {
            evicted_func_id = evicted.func_id;
        }

        inner.entries.insert(key, Arc::new(code));
        inner.insertion_order.push_back(key);
        evicted_func_id
    }

    /// Remove compiled code from the cache (e.g., on validation mismatch).
    pub fn invalidate(&self, key: &CacheKey) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut inner = self.inner.write().unwrap();
        inner.entries.remove(key);
        inner.insertion_order.retain(|k| k != key);
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let inner = self.inner.read().unwrap();
        inner.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove all entries from the cache.
    ///
    /// Used by `JitState::reset_for_testing()` to prevent state leakage
    /// between `#[serial]` tests.
    pub fn clear(&self) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut inner = self.inner.write().unwrap();
        inner.entries.clear();
        inner.insertion_order.clear();
    }
}

impl Default for CodeCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_fork() -> Fork {
        Fork::Cancun
    }

    #[test]
    fn test_cache_insert_and_get() {
        let cache = CodeCache::new();
        let key = (H256::zero(), default_fork());

        assert!(cache.get(&key).is_none());
        assert!(cache.is_empty());

        // SAFETY: null pointer is acceptable for testing metadata-only operations
        #[expect(unsafe_code)]
        let code = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None) };
        cache.insert(key, code);

        assert!(cache.get(&key).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_invalidate() {
        let cache = CodeCache::new();
        let key = (H256::zero(), default_fork());

        #[expect(unsafe_code)]
        let code = unsafe { CompiledCode::new(std::ptr::null(), 50, 3, None) };
        cache.insert(key, code);
        assert_eq!(cache.len(), 1);

        cache.invalidate(&key);
        assert!(cache.get(&key).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_eviction() {
        let cache = CodeCache::with_max_entries(3);

        let k1 = (H256::from_low_u64_be(1), default_fork());
        let k2 = (H256::from_low_u64_be(2), default_fork());
        let k3 = (H256::from_low_u64_be(3), default_fork());
        let k4 = (H256::from_low_u64_be(4), default_fork());

        // Insert 3 entries (at capacity)
        #[expect(unsafe_code)]
        let code1 = unsafe { CompiledCode::new(std::ptr::null(), 10, 1, None) };
        cache.insert(k1, code1);
        #[expect(unsafe_code)]
        let code2 = unsafe { CompiledCode::new(std::ptr::null(), 20, 2, None) };
        cache.insert(k2, code2);
        #[expect(unsafe_code)]
        let code3 = unsafe { CompiledCode::new(std::ptr::null(), 30, 3, None) };
        cache.insert(k3, code3);
        assert_eq!(cache.len(), 3);

        // Insert 4th entry → oldest (k1) should be evicted
        #[expect(unsafe_code)]
        let code4 = unsafe { CompiledCode::new(std::ptr::null(), 40, 4, None) };
        let evicted = cache.insert(k4, code4);
        assert!(evicted.is_none(), "evicted entry had no func_id");
        assert_eq!(cache.len(), 3);
        assert!(cache.get(&k1).is_none(), "oldest entry should be evicted");
        assert!(cache.get(&k2).is_some());
        assert!(cache.get(&k3).is_some());
        assert!(cache.get(&k4).is_some());
    }

    #[test]
    fn test_cache_update_existing_no_eviction() {
        let cache = CodeCache::with_max_entries(2);

        let k1 = (H256::from_low_u64_be(1), default_fork());
        let k2 = (H256::from_low_u64_be(2), default_fork());

        #[expect(unsafe_code)]
        let code1 = unsafe { CompiledCode::new(std::ptr::null(), 10, 1, None) };
        cache.insert(k1, code1);
        #[expect(unsafe_code)]
        let code2 = unsafe { CompiledCode::new(std::ptr::null(), 20, 2, None) };
        cache.insert(k2, code2);
        assert_eq!(cache.len(), 2);

        // Re-insert k1 with different metadata — should NOT evict
        #[expect(unsafe_code)]
        let code1_updated = unsafe { CompiledCode::new(std::ptr::null(), 100, 10, None) };
        cache.insert(k1, code1_updated);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&k1).is_some());
        assert!(cache.get(&k2).is_some());
    }

    #[test]
    fn test_cache_separate_fork_entries() {
        let cache = CodeCache::new();
        let hash = H256::from_low_u64_be(42);

        let key_cancun = (hash, Fork::Cancun);
        let key_prague = (hash, Fork::Prague);

        #[expect(unsafe_code)]
        let code_cancun = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None) };
        cache.insert(key_cancun, code_cancun);

        #[expect(unsafe_code)]
        let code_prague = unsafe { CompiledCode::new(std::ptr::null(), 100, 6, None) };
        cache.insert(key_prague, code_prague);

        assert_eq!(cache.len(), 2);

        let cancun_entry = cache.get(&key_cancun).expect("cancun entry should exist");
        let prague_entry = cache.get(&key_prague).expect("prague entry should exist");
        assert_eq!(cancun_entry.basic_block_count, 5);
        assert_eq!(prague_entry.basic_block_count, 6);
    }
}
