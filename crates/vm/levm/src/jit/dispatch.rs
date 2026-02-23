//! JIT dispatch logic.
//!
//! Provides the global JIT state, the dispatch check used by `vm.rs`
//! to determine whether a bytecode has been JIT-compiled, and the
//! `JitBackend` trait for dependency-inverted execution.

use std::sync::{Arc, RwLock};

use ethrex_common::H256;

use super::cache::{CodeCache, CompiledCode};
use super::counter::ExecutionCounter;
use super::types::{JitConfig, JitOutcome};
use crate::call_frame::CallFrame;
use crate::db::gen_db::GeneralizedDatabase;
use crate::environment::Environment;
use crate::vm::Substate;

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
    ) -> Result<JitOutcome, String>;
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
}

impl JitState {
    /// Create a new JIT state with default configuration.
    pub fn new() -> Self {
        Self {
            cache: CodeCache::new(),
            counter: ExecutionCounter::new(),
            config: JitConfig::default(),
            backend: RwLock::new(None),
        }
    }

    /// Create a new JIT state with a specific configuration.
    pub fn with_config(config: JitConfig) -> Self {
        Self {
            cache: CodeCache::new(),
            counter: ExecutionCounter::new(),
            config,
            backend: RwLock::new(None),
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
    ) -> Option<Result<JitOutcome, String>> {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let guard = self.backend.read().unwrap();
        let backend = guard.as_ref()?;
        Some(backend.execute(compiled, call_frame, db, substate, env))
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
