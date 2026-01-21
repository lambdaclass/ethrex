//! Global JIT code cache with persistence support.
//!
//! This module provides a thread-safe global cache for JIT-compiled code
//! that can be populated from storage and persisted after execution.

use ethrex_common::H256;
use rustc_hash::FxHashMap;
use std::sync::RwLock;

use super::compiler::{JitCode, JitCompiler, JitError};
use super::persistence::SerializedJitCode;

/// Global JIT code cache.
///
/// This is a singleton cache shared across all VM instances in a process.
/// It can be populated from storage at startup and persisted after execution.
static GLOBAL_CACHE: RwLock<Option<JitCache>> = RwLock::new(None);

/// JIT code cache.
pub struct JitCache {
    /// Compiled code indexed by bytecode hash.
    cache: FxHashMap<H256, JitCode>,
    /// Newly compiled entries that need to be persisted.
    dirty: FxHashMap<H256, SerializedJitCode>,
}

impl JitCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            cache: FxHashMap::default(),
            dirty: FxHashMap::default(),
        }
    }

    /// Get JIT code for a bytecode hash.
    pub fn get(&self, hash: &H256) -> Option<&JitCode> {
        self.cache.get(hash)
    }

    /// Insert compiled code into the cache.
    ///
    /// Also marks the entry as dirty for persistence.
    pub fn insert(&mut self, hash: H256, code: JitCode) -> Result<(), JitError> {
        // Serialize for persistence
        let serialized = code.serialize()?;
        self.dirty.insert(hash, serialized);
        self.cache.insert(hash, code);
        Ok(())
    }

    /// Load a serialized entry from storage.
    ///
    /// Returns Ok(true) if loaded successfully, Ok(false) if already present.
    pub fn load_from_bytes(&mut self, hash: H256, bytes: &[u8]) -> Result<bool, JitError> {
        if self.cache.contains_key(&hash) {
            return Ok(false);
        }

        let serialized = SerializedJitCode::from_bytes(bytes)
            .map_err(|_| JitError::InvalidBytecode)?;
        let code = JitCode::deserialize(&serialized)?;
        self.cache.insert(hash, code);
        Ok(true)
    }

    /// Get or compile JIT code for bytecode.
    ///
    /// If the code is not in the cache, compiles it and adds to cache.
    pub fn get_or_compile(
        &mut self,
        hash: H256,
        bytecode: &[u8],
        compiler: &JitCompiler,
    ) -> Result<&JitCode, JitError> {
        if !self.cache.contains_key(&hash) {
            let code = compiler.compile(bytecode)?;
            self.insert(hash, code)?;
        }
        self.cache.get(&hash).ok_or(JitError::InvalidBytecode)
    }

    /// Take dirty entries that need to be persisted.
    ///
    /// Clears the dirty set after returning.
    pub fn take_dirty(&mut self) -> Vec<(H256, Vec<u8>)> {
        let dirty: Vec<_> = self
            .dirty
            .drain()
            .filter_map(|(hash, serialized)| {
                serialized.to_bytes().ok().map(|bytes| (hash, bytes))
            })
            .collect();
        dirty
    }

    /// Check if there are dirty entries.
    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Get the number of cached entries.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Default for JitCache {
    fn default() -> Self {
        Self::new()
    }
}

// Global cache functions

/// Initialize the global JIT cache.
///
/// Should be called once at startup. Returns an error if already initialized.
pub fn init_global_cache() -> Result<(), &'static str> {
    let mut guard = GLOBAL_CACHE.write().map_err(|_| "Failed to acquire lock")?;
    if guard.is_some() {
        return Err("Global cache already initialized");
    }
    *guard = Some(JitCache::new());
    Ok(())
}

/// Get a reference to the global cache for reading.
///
/// Initializes the cache if not already initialized.
pub fn with_global_cache<F, R>(f: F) -> Result<R, &'static str>
where
    F: FnOnce(&JitCache) -> R,
{
    let guard = GLOBAL_CACHE.read().map_err(|_| "Failed to acquire lock")?;
    match guard.as_ref() {
        Some(cache) => Ok(f(cache)),
        None => Err("Global cache not initialized"),
    }
}

/// Get a mutable reference to the global cache.
///
/// Initializes the cache if not already initialized.
pub fn with_global_cache_mut<F, R>(f: F) -> Result<R, &'static str>
where
    F: FnOnce(&mut JitCache) -> R,
{
    let mut guard = GLOBAL_CACHE.write().map_err(|_| "Failed to acquire lock")?;
    if guard.is_none() {
        *guard = Some(JitCache::new());
    }
    match guard.as_mut() {
        Some(cache) => Ok(f(cache)),
        None => Err("Failed to create cache"),
    }
}

/// Load JIT code from bytes into the global cache.
pub fn load_jit_code(hash: H256, bytes: &[u8]) -> Result<bool, JitError> {
    with_global_cache_mut(|cache| cache.load_from_bytes(hash, bytes))
        .map_err(|_| JitError::Disabled)?
}

/// Get or compile JIT code from the global cache.
pub fn get_or_compile_jit_code(
    hash: H256,
    bytecode: &[u8],
    compiler: &JitCompiler,
) -> Result<(), JitError> {
    with_global_cache_mut(|cache| {
        let _ = cache.get_or_compile(hash, bytecode, compiler)?;
        Ok(())
    })
    .map_err(|_| JitError::Disabled)?
}

/// Take dirty entries from the global cache for persistence.
pub fn take_dirty_entries() -> Result<Vec<(H256, Vec<u8>)>, &'static str> {
    with_global_cache_mut(|cache| cache.take_dirty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic() {
        let mut cache = JitCache::new();
        assert!(cache.is_empty());

        let compiler = JitCompiler::new();
        let bytecode = [0x00]; // STOP
        let hash_bytes = ethrex_crypto::keccak::keccak_hash(&bytecode);
        let hash = H256::from_slice(&hash_bytes);

        // Compile and insert
        let result = cache.get_or_compile(hash, &bytecode, &compiler);
        assert!(result.is_ok());
        assert_eq!(cache.len(), 1);
        assert!(cache.has_dirty());

        // Should be cached now
        assert!(cache.get(&hash).is_some());

        // Take dirty entries
        let dirty = cache.take_dirty();
        assert_eq!(dirty.len(), 1);
        assert!(!cache.has_dirty());
    }

    #[test]
    fn test_cache_load_from_bytes() {
        let compiler = JitCompiler::new();
        let bytecode = [0x00]; // STOP
        let hash_bytes = ethrex_crypto::keccak::keccak_hash(&bytecode);
        let hash = H256::from_slice(&hash_bytes);

        // Compile to get serialized form
        let code = compiler.compile(&bytecode).expect("compile");
        let serialized = code.serialize().expect("serialize");
        let bytes = serialized.to_bytes().expect("to_bytes");

        // Load into new cache
        let mut cache = JitCache::new();
        let loaded = cache.load_from_bytes(hash, &bytes).expect("load");
        assert!(loaded);
        assert!(cache.get(&hash).is_some());

        // Loading again should return false (already present)
        let loaded_again = cache.load_from_bytes(hash, &bytes).expect("load again");
        assert!(!loaded_again);
    }
}
