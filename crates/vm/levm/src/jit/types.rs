//! JIT compilation types.
//!
//! Core data structures for the tiered JIT compilation system.
//! All types are designed to be lightweight — no external dependencies beyond std.

use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use ethrex_common::{Address, H256, U256};

/// Configuration for the JIT compilation tier.
#[derive(Debug, Clone)]
pub struct JitConfig {
    /// Number of executions before a contract becomes a compilation candidate.
    pub compilation_threshold: u64,
    /// When true, JIT executions are logged for offline validation.
    /// Should always be true during PoC; can be relaxed in production.
    pub validation_mode: bool,
    /// Maximum bytecode size eligible for JIT compilation (EIP-170: 24576).
    pub max_bytecode_size: usize,
    /// Maximum number of compiled bytecodes to keep in the cache.
    /// Oldest entries are evicted when this limit is reached.
    pub max_cache_entries: usize,
    /// Number of JIT executions to validate per (bytecode, fork) pair.
    /// After this many validations succeed, the bytecode is considered trusted.
    pub max_validation_runs: u64,
    /// Number of functions per arena (default 64).
    pub arena_capacity: u16,
    /// Maximum number of concurrent arenas (default 32, i.e. 2048 functions).
    pub max_arenas: usize,
    /// Maximum JIT memory usage in megabytes (RSS ceiling, default 512).
    pub max_memory_mb: usize,
    /// Number of background compilation worker threads (default: num_cpus / 2, min 1).
    pub compile_workers: usize,
    /// Enable JIT-to-JIT dispatch: when a child bytecode is in the JIT cache,
    /// execute it via JIT directly instead of falling back to the interpreter.
    /// Default: true.
    pub enable_jit_dispatch: bool,
}

impl JitConfig {
    /// Check if a bytecode length exceeds the JIT compilation size limit.
    pub fn is_bytecode_oversized(&self, len: usize) -> bool {
        len > self.max_bytecode_size
    }
}

impl Default for JitConfig {
    fn default() -> Self {
        Self {
            compilation_threshold: 10,
            validation_mode: true,
            max_bytecode_size: 24576,
            max_cache_entries: 1024,
            max_validation_runs: 3,
            arena_capacity: 64,
            max_arenas: 32,
            max_memory_mb: 512,
            compile_workers: 1, // Default 1; tokamak-jit overrides with num_cpus/2
            enable_jit_dispatch: true,
        }
    }
}

/// Opaque state for resuming JIT execution after a sub-call.
///
/// Constructed by `tokamak-jit` when JIT code hits CALL/CREATE, consumed
/// by `execute_resume` when the sub-call completes.
pub struct JitResumeState(pub Box<dyn std::any::Any + Send>);

impl std::fmt::Debug for JitResumeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JitResumeState").finish_non_exhaustive()
    }
}

/// Result of a sub-call executed by the LEVM interpreter on behalf of JIT.
#[derive(Debug, Clone)]
pub struct SubCallResult {
    /// Whether the sub-call succeeded.
    pub success: bool,
    /// Gas limit that was allocated to the sub-call (from the FrameInput).
    /// Used to compute unused gas to credit back to the JIT parent.
    pub gas_limit: u64,
    /// Gas consumed by the sub-call.
    pub gas_used: u64,
    /// Output data from the sub-call.
    pub output: Bytes,
    /// For CREATE: the created contract address (if success).
    pub created_address: Option<Address>,
}

/// Sub-call request from JIT-compiled code, translated to LEVM types.
#[derive(Debug)]
pub enum JitSubCall {
    /// CALL/CALLCODE/DELEGATECALL/STATICCALL from JIT code.
    Call {
        gas_limit: u64,
        caller: Address,
        target: Address,
        code_address: Address,
        value: U256,
        calldata: Bytes,
        is_static: bool,
        scheme: JitCallScheme,
        return_offset: usize,
        return_size: usize,
    },
    /// CREATE/CREATE2 from JIT code.
    Create {
        gas_limit: u64,
        caller: Address,
        value: U256,
        init_code: Bytes,
        /// Some for CREATE2, None for CREATE.
        salt: Option<U256>,
    },
}

/// Call scheme variants for JIT sub-calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitCallScheme {
    Call,
    CallCode,
    DelegateCall,
    StaticCall,
}

/// Outcome of a JIT-compiled execution.
#[derive(Debug)]
pub enum JitOutcome {
    /// Execution succeeded.
    Success { gas_used: u64, output: Bytes },
    /// Execution reverted (REVERT opcode).
    Revert { gas_used: u64, output: Bytes },
    /// Bytecode was not compiled (fall through to interpreter).
    NotCompiled,
    /// JIT execution error (fall through to interpreter).
    Error(String),
    /// JIT code hit a CALL/CREATE and is suspended, waiting for the sub-call result.
    Suspended {
        resume_state: JitResumeState,
        sub_call: JitSubCall,
    },
}

/// Pre-analyzed bytecode metadata used for compilation decisions and basic block mapping.
#[derive(Debug, Clone)]
pub struct AnalyzedBytecode {
    /// Keccak hash of the bytecode (used as cache key).
    pub hash: H256,
    /// Raw bytecode bytes.
    pub bytecode: Bytes,
    /// Valid JUMPDEST positions (reused from LEVM's `Code::jump_targets`).
    pub jump_targets: Vec<u32>,
    /// Basic block boundaries as (start, end) byte offsets.
    /// A basic block starts at a JUMPDEST or byte 0, and ends at
    /// JUMP/JUMPI/STOP/RETURN/REVERT/INVALID or the end of bytecode.
    pub basic_blocks: Vec<(usize, usize)>,
    /// Total number of opcodes in the bytecode.
    pub opcode_count: usize,
    /// Whether the bytecode contains CALL/CALLCODE/DELEGATECALL/STATICCALL/CREATE/CREATE2.
    /// Bytecodes with external calls are skipped by the JIT compiler in Phase 4.
    pub has_external_calls: bool,
}

/// Atomic metrics for JIT compilation and execution events.
#[derive(Debug)]
pub struct JitMetrics {
    /// Number of successful JIT executions.
    pub jit_executions: AtomicU64,
    /// Number of JIT fallbacks to interpreter.
    pub jit_fallbacks: AtomicU64,
    /// Number of successful compilations.
    pub compilations: AtomicU64,
    /// Number of compilation skips (e.g., external calls detected).
    pub compilation_skips: AtomicU64,
    /// Number of successful dual-execution validations (JIT matched interpreter).
    pub validation_successes: AtomicU64,
    /// Number of dual-execution validation mismatches (JIT diverged from interpreter).
    pub validation_mismatches: AtomicU64,
    /// Number of arenas created (G-1 arena lifecycle).
    pub arenas_created: AtomicU64,
    /// Number of arenas freed (G-1 arena lifecycle).
    pub arenas_freed: AtomicU64,
    /// Number of compiled functions evicted from the cache.
    pub functions_evicted: AtomicU64,
    /// Number of successful JIT-to-JIT child dispatches (child executed via JIT instead of interpreter).
    pub jit_to_jit_dispatches: AtomicU64,
}

impl JitMetrics {
    /// Create a new metrics instance with all counters at zero.
    pub fn new() -> Self {
        Self {
            jit_executions: AtomicU64::new(0),
            jit_fallbacks: AtomicU64::new(0),
            compilations: AtomicU64::new(0),
            compilation_skips: AtomicU64::new(0),
            validation_successes: AtomicU64::new(0),
            validation_mismatches: AtomicU64::new(0),
            arenas_created: AtomicU64::new(0),
            arenas_freed: AtomicU64::new(0),
            functions_evicted: AtomicU64::new(0),
            jit_to_jit_dispatches: AtomicU64::new(0),
        }
    }

    /// Reset all counters to zero.
    ///
    /// Used by `JitState::reset_for_testing()` to prevent state leakage
    /// between `#[serial]` tests. Not available in production builds.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn reset(&self) {
        self.jit_executions.store(0, Ordering::Relaxed);
        self.jit_fallbacks.store(0, Ordering::Relaxed);
        self.compilations.store(0, Ordering::Relaxed);
        self.compilation_skips.store(0, Ordering::Relaxed);
        self.validation_successes.store(0, Ordering::Relaxed);
        self.validation_mismatches.store(0, Ordering::Relaxed);
        self.arenas_created.store(0, Ordering::Relaxed);
        self.arenas_freed.store(0, Ordering::Relaxed);
        self.functions_evicted.store(0, Ordering::Relaxed);
        self.jit_to_jit_dispatches.store(0, Ordering::Relaxed);
    }

    /// Get a snapshot of all metrics.
    pub fn snapshot(&self) -> (u64, u64, u64, u64, u64, u64, u64) {
        (
            self.jit_executions.load(Ordering::Relaxed),
            self.jit_fallbacks.load(Ordering::Relaxed),
            self.compilations.load(Ordering::Relaxed),
            self.compilation_skips.load(Ordering::Relaxed),
            self.validation_successes.load(Ordering::Relaxed),
            self.validation_mismatches.load(Ordering::Relaxed),
            self.jit_to_jit_dispatches.load(Ordering::Relaxed),
        )
    }
}

impl Default for JitMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_reset() {
        let metrics = JitMetrics::new();
        metrics.jit_executions.store(10, Ordering::Relaxed);
        metrics.jit_fallbacks.store(5, Ordering::Relaxed);
        metrics.compilations.store(3, Ordering::Relaxed);
        metrics.compilation_skips.store(2, Ordering::Relaxed);
        metrics.validation_successes.store(7, Ordering::Relaxed);
        metrics.validation_mismatches.store(1, Ordering::Relaxed);

        assert_eq!(metrics.snapshot(), (10, 5, 3, 2, 7, 1, 0));

        metrics.reset();

        assert_eq!(metrics.snapshot(), (0, 0, 0, 0, 0, 0, 0));
    }
}
