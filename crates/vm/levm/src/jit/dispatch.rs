//! JIT dispatch logic.
//!
//! Provides the global JIT state, the dispatch check used by `vm.rs`
//! to determine whether a bytecode has been JIT-compiled, and the
//! `JitBackend` trait for dependency-inverted execution.

use std::sync::{Arc, RwLock};

use ethrex_common::{H256, U256};
use rustc_hash::FxHashMap;

use super::cache::{CodeCache, CompiledCode};
use super::counter::ExecutionCounter;
use super::types::{JitConfig, JitMetrics, JitOutcome};
use crate::call_frame::CallFrame;
use crate::db::gen_db::GeneralizedDatabase;
use crate::environment::Environment;
use crate::vm::Substate;

/// Type alias for the storage original values map used in SSTORE gas calculation.
pub type StorageOriginalValues = FxHashMap<(ethrex_common::Address, H256), U256>;

/// Trait for JIT execution backends.
///
/// LEVM defines this interface; `tokamak-jit` provides the implementation.
/// This dependency inversion prevents LEVM from depending on heavy LLVM/revmc
/// crates while still allowing JIT-compiled code to execute through the VM.
pub trait JitBackend: Send + Sync {
    /// Execute JIT-compiled code against the given LEVM state.
    fn execute(
        &self,
        compiled: &CompiledCode,
        call_frame: &mut CallFrame,
        db: &mut GeneralizedDatabase,
        substate: &mut Substate,
        env: &Environment,
        storage_original_values: &mut StorageOriginalValues,
    ) -> Result<JitOutcome, String>;

    /// Compile bytecode and insert the result into the cache.
    ///
    /// Called when the execution counter reaches the compilation threshold.
    /// Returns `Ok(())` on success or an error message on failure.
    fn compile(&self, code: &ethrex_common::types::Code, cache: &CodeCache)
        -> Result<(), String>;
}

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
    /// Registered JIT execution backend (set by `tokamak-jit` at startup).
    backend: RwLock<Option<Arc<dyn JitBackend>>>,
    /// Atomic metrics for monitoring JIT activity.
    pub metrics: JitMetrics,
}

impl JitState {
    /// Create a new JIT state with default configuration.
    pub fn new() -> Self {
        let config = JitConfig::default();
        let cache = CodeCache::with_max_entries(config.max_cache_entries);
        Self {
            cache,
            counter: ExecutionCounter::new(),
            config,
            backend: RwLock::new(None),
            metrics: JitMetrics::new(),
        }
    }

    /// Create a new JIT state with a specific configuration.
    pub fn with_config(config: JitConfig) -> Self {
        let cache = CodeCache::with_max_entries(config.max_cache_entries);
        Self {
            cache,
            counter: ExecutionCounter::new(),
            config,
            backend: RwLock::new(None),
            metrics: JitMetrics::new(),
        }
    }

    /// Register a JIT execution backend.
    ///
    /// Call this once at application startup (from `tokamak-jit`) to enable
    /// JIT execution. Without a registered backend, JIT dispatch is a no-op.
    pub fn register_backend(&self, backend: Arc<dyn JitBackend>) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut guard = self.backend.write().unwrap();
        *guard = Some(backend);
    }

    /// Execute JIT-compiled code through the registered backend.
    ///
    /// Returns `None` if no backend is registered, otherwise returns the
    /// execution result.
    pub fn execute_jit(
        &self,
        compiled: &CompiledCode,
        call_frame: &mut CallFrame,
        db: &mut GeneralizedDatabase,
        substate: &mut Substate,
        env: &Environment,
        storage_original_values: &mut StorageOriginalValues,
    ) -> Option<Result<JitOutcome, String>> {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let guard = self.backend.read().unwrap();
        let backend = guard.as_ref()?;
        Some(backend.execute(
            compiled,
            call_frame,
            db,
            substate,
            env,
            storage_original_values,
        ))
    }

    /// Get a reference to the registered backend (if any).
    pub fn backend(&self) -> Option<Arc<dyn JitBackend>> {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let guard = self.backend.read().unwrap();
        guard.clone()
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
