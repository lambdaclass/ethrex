//! JIT dispatch logic.
//!
//! Provides the global JIT state, the dispatch check used by `vm.rs`
//! to determine whether a bytecode has been JIT-compiled, and the
//! `JitBackend` trait for dependency-inverted execution.

use std::sync::{Arc, RwLock};

use ethrex_common::types::Fork;
use ethrex_common::{H256, U256};
use rustc_hash::{FxHashMap, FxHashSet};

use super::cache::{CacheKey, CodeCache, CompiledCode};
use super::compiler_thread::{CompilationRequest, CompilerThread};
use super::counter::ExecutionCounter;
use super::types::{JitConfig, JitMetrics, JitOutcome, JitResumeState, SubCallResult};
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

    /// Resume JIT execution after a sub-call completes.
    ///
    /// Called when the outer JIT code was suspended for a CALL/CREATE,
    /// the sub-call has been executed by the LEVM interpreter, and we
    /// need to feed the result back and continue JIT execution.
    #[allow(clippy::too_many_arguments)]
    fn execute_resume(
        &self,
        resume_state: JitResumeState,
        sub_result: SubCallResult,
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
    fn compile(
        &self,
        code: &ethrex_common::types::Code,
        fork: Fork,
        cache: &CodeCache,
    ) -> Result<(), String>;
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
    /// Background compilation thread (set by `tokamak-jit` at startup).
    compiler_thread: RwLock<Option<CompilerThread>>,
    /// Per-(hash, fork) validation run counter for output-only validation.
    validation_counts: RwLock<FxHashMap<CacheKey, u64>>,
    /// Bytecodes known to exceed `max_bytecode_size` — negative cache to
    /// avoid repeated size checks and compilation attempts.
    oversized_hashes: RwLock<FxHashSet<H256>>,
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
            compiler_thread: RwLock::new(None),
            validation_counts: RwLock::new(FxHashMap::default()),
            oversized_hashes: RwLock::new(FxHashSet::default()),
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
            compiler_thread: RwLock::new(None),
            validation_counts: RwLock::new(FxHashMap::default()),
            oversized_hashes: RwLock::new(FxHashSet::default()),
        }
    }

    /// Reset all mutable state for test isolation.
    ///
    /// Must be called at the start of every `#[serial]` JIT test to prevent
    /// state accumulated by prior tests (cache entries, execution counts,
    /// metrics, validation counts) from leaking into subsequent tests.
    ///
    /// This does NOT reset `config` (immutable) or destroy the LLVM context
    /// held by the backend — it only clears the runtime accumulators.
    /// Not available in production builds.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn reset_for_testing(&self) {
        self.cache.clear();
        self.counter.clear();
        self.metrics.reset();
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        {
            *self.backend.write().unwrap() = None;
        }
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        {
            *self.compiler_thread.write().unwrap() = None;
        }
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        {
            self.validation_counts.write().unwrap().clear();
        }
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        {
            self.oversized_hashes.write().unwrap().clear();
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

    /// Register the background compiler thread.
    ///
    /// Call this once at application startup (from `tokamak-jit`) to enable
    /// background compilation. Without a registered thread, compilation
    /// happens synchronously on the VM thread.
    pub fn register_compiler_thread(&self, thread: CompilerThread) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut guard = self.compiler_thread.write().unwrap();
        *guard = Some(thread);
    }

    /// Send a compilation request to the background thread.
    ///
    /// Returns `true` if the request was queued, `false` if no thread is
    /// registered or the channel is disconnected (falls through to sync compile).
    pub fn request_compilation(&self, code: ethrex_common::types::Code, fork: Fork) -> bool {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let guard = self.compiler_thread.read().unwrap();
        match guard.as_ref() {
            Some(thread) => thread.send(CompilationRequest { code, fork }),
            None => false,
        }
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

    /// Resume JIT execution after a sub-call through the registered backend.
    ///
    /// Returns `None` if no backend is registered, otherwise returns the
    /// execution result (which may be another `Suspended`).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_jit_resume(
        &self,
        resume_state: JitResumeState,
        sub_result: SubCallResult,
        call_frame: &mut CallFrame,
        db: &mut GeneralizedDatabase,
        substate: &mut Substate,
        env: &Environment,
        storage_original_values: &mut StorageOriginalValues,
    ) -> Option<Result<JitOutcome, String>> {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let guard = self.backend.read().unwrap();
        let backend = guard.as_ref()?;
        Some(backend.execute_resume(
            resume_state,
            sub_result,
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

    /// Check if this (hash, fork) pair should be validated.
    ///
    /// Returns `true` if the validation count for this key is below
    /// `max_validation_runs`, meaning we should log the JIT outcome.
    pub fn should_validate(&self, key: &CacheKey) -> bool {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let counts = self.validation_counts.read().unwrap();
        let count = counts.get(key).copied().unwrap_or(0);
        count < self.config.max_validation_runs
    }

    /// Check if a bytecode hash is known to be oversized.
    ///
    /// Returns `true` if the bytecode was previously marked via [`mark_oversized`].
    /// Uses a read-lock on a small `FxHashSet` — negligible overhead.
    pub fn is_oversized(&self, hash: &H256) -> bool {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let guard = self.oversized_hashes.read().unwrap();
        guard.contains(hash)
    }

    /// Mark a bytecode hash as oversized (too large for JIT compilation).
    ///
    /// Subsequent calls to [`is_oversized`] for this hash will return `true`,
    /// allowing the VM dispatch to skip JIT entirely.
    pub fn mark_oversized(&self, hash: H256) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut guard = self.oversized_hashes.write().unwrap();
        guard.insert(hash);
    }

    /// Record that a validation run occurred for this (hash, fork) pair.
    pub fn record_validation(&self, key: &CacheKey) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut counts = self.validation_counts.write().unwrap();
        let count = counts.entry(*key).or_insert(0);
        *count = count.saturating_add(1);
    }
}

impl Default for JitState {
    fn default() -> Self {
        Self::new()
    }
}

/// Check the JIT cache for compiled code matching the given bytecode hash and fork.
///
/// Returns `Some(compiled)` if the bytecode has been JIT-compiled for this fork,
/// `None` otherwise (caller should fall through to interpreter).
pub fn try_jit_dispatch(
    state: &JitState,
    bytecode_hash: &H256,
    fork: Fork,
) -> Option<Arc<CompiledCode>> {
    state.cache.get(&(*bytecode_hash, fork))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oversized_default_empty() {
        let state = JitState::new();
        let hash = H256::from_low_u64_be(0x42);
        assert!(!state.is_oversized(&hash));
    }

    #[test]
    fn test_mark_and_check_oversized() {
        let state = JitState::new();
        let hash = H256::from_low_u64_be(0x42);
        state.mark_oversized(hash);
        assert!(state.is_oversized(&hash));
    }

    #[test]
    fn test_oversized_does_not_affect_other_hashes() {
        let state = JitState::new();
        let h1 = H256::from_low_u64_be(0x01);
        let h2 = H256::from_low_u64_be(0x02);
        state.mark_oversized(h1);
        assert!(state.is_oversized(&h1));
        assert!(!state.is_oversized(&h2));
    }

    #[test]
    fn test_oversized_reset_clears() {
        let state = JitState::new();
        let hash = H256::from_low_u64_be(0x42);
        state.mark_oversized(hash);
        assert!(state.is_oversized(&hash));
        state.reset_for_testing();
        assert!(!state.is_oversized(&hash));
    }
}
