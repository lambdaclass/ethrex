//! JIT code cache.
//!
//! Stores compiled function pointers keyed by bytecode hash.
//! The cache is thread-safe and designed for concurrent read access
//! with infrequent writes (compilation events).

use ethrex_common::H256;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

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
    pub unsafe fn new(ptr: *const (), bytecode_size: usize, basic_block_count: usize) -> Self {
        Self {
            ptr,
            bytecode_size,
            basic_block_count,
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
            .finish()
    }
}

/// Inner state for the code cache (behind RwLock).
#[derive(Debug)]
struct CodeCacheInner {
    entries: HashMap<H256, Arc<CompiledCode>>,
    insertion_order: VecDeque<H256>,
    max_entries: usize,
}

/// Thread-safe cache of JIT-compiled bytecodes with LRU eviction.
///
/// When the cache reaches `max_entries`, the oldest entry (by insertion time)
/// is evicted. Note: LLVM JIT memory is NOT freed on eviction (revmc limitation).
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

    /// Look up compiled code by bytecode hash.
    pub fn get(&self, hash: &H256) -> Option<Arc<CompiledCode>> {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let inner = self.inner.read().unwrap();
        inner.entries.get(hash).cloned()
    }

    /// Insert compiled code into the cache, evicting the oldest entry if at capacity.
    pub fn insert(&self, hash: H256, code: CompiledCode) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut inner = self.inner.write().unwrap();

        // If already present, just update the value (no eviction needed)
        if let std::collections::hash_map::Entry::Occupied(mut e) = inner.entries.entry(hash) {
            e.insert(Arc::new(code));
            return;
        }

        // Evict oldest if at capacity
        if inner.max_entries > 0
            && inner.entries.len() >= inner.max_entries
            && let Some(oldest) = inner.insertion_order.pop_front()
        {
            inner.entries.remove(&oldest);
        }

        inner.entries.insert(hash, Arc::new(code));
        inner.insertion_order.push_back(hash);
    }

    /// Remove compiled code from the cache (e.g., on validation mismatch).
    pub fn invalidate(&self, hash: &H256) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut inner = self.inner.write().unwrap();
        inner.entries.remove(hash);
        inner.insertion_order.retain(|h| h != hash);
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
}

impl Default for CodeCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_and_get() {
        let cache = CodeCache::new();
        let hash = H256::zero();

        assert!(cache.get(&hash).is_none());
        assert!(cache.is_empty());

        // SAFETY: null pointer is acceptable for testing metadata-only operations
        #[expect(unsafe_code)]
        let code = unsafe { CompiledCode::new(std::ptr::null(), 100, 5) };
        cache.insert(hash, code);

        assert!(cache.get(&hash).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_invalidate() {
        let cache = CodeCache::new();
        let hash = H256::zero();

        #[expect(unsafe_code)]
        let code = unsafe { CompiledCode::new(std::ptr::null(), 50, 3) };
        cache.insert(hash, code);
        assert_eq!(cache.len(), 1);

        cache.invalidate(&hash);
        assert!(cache.get(&hash).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_eviction() {
        let cache = CodeCache::with_max_entries(3);

        let h1 = H256::from_low_u64_be(1);
        let h2 = H256::from_low_u64_be(2);
        let h3 = H256::from_low_u64_be(3);
        let h4 = H256::from_low_u64_be(4);

        // Insert 3 entries (at capacity)
        #[expect(unsafe_code)]
        let code1 = unsafe { CompiledCode::new(std::ptr::null(), 10, 1) };
        cache.insert(h1, code1);
        #[expect(unsafe_code)]
        let code2 = unsafe { CompiledCode::new(std::ptr::null(), 20, 2) };
        cache.insert(h2, code2);
        #[expect(unsafe_code)]
        let code3 = unsafe { CompiledCode::new(std::ptr::null(), 30, 3) };
        cache.insert(h3, code3);
        assert_eq!(cache.len(), 3);

        // Insert 4th entry → oldest (h1) should be evicted
        #[expect(unsafe_code)]
        let code4 = unsafe { CompiledCode::new(std::ptr::null(), 40, 4) };
        cache.insert(h4, code4);
        assert_eq!(cache.len(), 3);
        assert!(cache.get(&h1).is_none(), "oldest entry should be evicted");
        assert!(cache.get(&h2).is_some());
        assert!(cache.get(&h3).is_some());
        assert!(cache.get(&h4).is_some());
    }

    #[test]
    fn test_cache_update_existing_no_eviction() {
        let cache = CodeCache::with_max_entries(2);

        let h1 = H256::from_low_u64_be(1);
        let h2 = H256::from_low_u64_be(2);

        #[expect(unsafe_code)]
        let code1 = unsafe { CompiledCode::new(std::ptr::null(), 10, 1) };
        cache.insert(h1, code1);
        #[expect(unsafe_code)]
        let code2 = unsafe { CompiledCode::new(std::ptr::null(), 20, 2) };
        cache.insert(h2, code2);
        assert_eq!(cache.len(), 2);

        // Re-insert h1 with different metadata — should NOT evict
        #[expect(unsafe_code)]
        let code1_updated = unsafe { CompiledCode::new(std::ptr::null(), 100, 10) };
        cache.insert(h1, code1_updated);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&h1).is_some());
        assert!(cache.get(&h2).is_some());
    }
}
