//! JIT dispatch logic.
//!
//! Provides the global JIT state and the dispatch check used by `vm.rs`
//! to determine whether a bytecode has been JIT-compiled.

use std::sync::Arc;

use ethrex_common::H256;

use super::cache::{CodeCache, CompiledCode};
use super::counter::ExecutionCounter;
use super::types::JitConfig;

/// Global JIT state shared across all VM instances.
///
/// This is initialized lazily (via `lazy_static`) and shared by reference
/// in `vm.rs`. The `tokamak-jit` crate populates the cache; LEVM only reads it.
pub struct JitState {
    /// Cache of JIT-compiled function pointers.
    pub cache: CodeCache,
    /// Per-bytecode execution counter for tiering decisions.
    pub counter: ExecutionCounter,
    /// JIT configuration.
    pub config: JitConfig,
}

impl JitState {
    /// Create a new JIT state with default configuration.
    pub fn new() -> Self {
        Self {
            cache: CodeCache::new(),
            counter: ExecutionCounter::new(),
            config: JitConfig::default(),
        }
    }

    /// Create a new JIT state with a specific configuration.
    pub fn with_config(config: JitConfig) -> Self {
        Self {
            cache: CodeCache::new(),
            counter: ExecutionCounter::new(),
            config,
        }
    }
}

impl Default for JitState {
    fn default() -> Self {
        Self::new()
    }
}

/// Check the JIT cache for compiled code matching the given bytecode hash.
///
/// Returns `Some(compiled)` if the bytecode has been JIT-compiled,
/// `None` otherwise (caller should fall through to interpreter).
pub fn try_jit_dispatch(state: &JitState, bytecode_hash: &H256) -> Option<Arc<CompiledCode>> {
    state.cache.get(bytecode_hash)
}
