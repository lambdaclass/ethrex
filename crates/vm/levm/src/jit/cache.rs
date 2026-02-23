//! JIT code cache.
//!
//! Stores compiled function pointers keyed by bytecode hash.
//! The cache is thread-safe and designed for concurrent read access
//! with infrequent writes (compilation events).

use ethrex_common::H256;
use std::collections::HashMap;
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

/// Thread-safe cache of JIT-compiled bytecodes.
#[derive(Debug, Clone)]
pub struct CodeCache {
    entries: Arc<RwLock<HashMap<H256, Arc<CompiledCode>>>>,
}

impl CodeCache {
    /// Create a new empty code cache.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Look up compiled code by bytecode hash.
    pub fn get(&self, hash: &H256) -> Option<Arc<CompiledCode>> {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let entries = self.entries.read().unwrap();
        entries.get(hash).cloned()
    }

    /// Insert compiled code into the cache.
    pub fn insert(&self, hash: H256, code: CompiledCode) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut entries = self.entries.write().unwrap();
        entries.insert(hash, Arc::new(code));
    }

    /// Remove compiled code from the cache (e.g., on validation mismatch).
    pub fn invalidate(&self, hash: &H256) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut entries = self.entries.write().unwrap();
        entries.remove(hash);
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let entries = self.entries.read().unwrap();
        entries.len()
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
}
