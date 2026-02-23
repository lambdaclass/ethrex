//! JIT compilation types.
//!
//! Core data structures for the tiered JIT compilation system.
//! All types are designed to be lightweight — no external dependencies beyond std.

use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use ethrex_common::H256;

/// Configuration for the JIT compilation tier.
#[derive(Debug, Clone)]
pub struct JitConfig {
    /// Number of executions before a contract becomes a compilation candidate.
    pub compilation_threshold: u64,
    /// When true, every JIT execution is validated against the interpreter.
    /// Should always be true during PoC; can be relaxed in production.
    pub validation_mode: bool,
    /// Maximum bytecode size eligible for JIT compilation (EIP-170: 24576).
    pub max_bytecode_size: usize,
    /// Maximum number of compiled bytecodes to keep in the cache.
    /// Oldest entries are evicted when this limit is reached.
    pub max_cache_entries: usize,
}

impl Default for JitConfig {
    fn default() -> Self {
        Self {
            compilation_threshold: 10,
            validation_mode: true,
            max_bytecode_size: 24576,
            max_cache_entries: 1024,
        }
    }
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
}

impl JitMetrics {
    /// Create a new metrics instance with all counters at zero.
    pub fn new() -> Self {
        Self {
            jit_executions: AtomicU64::new(0),
            jit_fallbacks: AtomicU64::new(0),
            compilations: AtomicU64::new(0),
            compilation_skips: AtomicU64::new(0),
        }
    }

    /// Get a snapshot of all metrics.
    pub fn snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.jit_executions.load(Ordering::Relaxed),
            self.jit_fallbacks.load(Ordering::Relaxed),
            self.compilations.load(Ordering::Relaxed),
            self.compilation_skips.load(Ordering::Relaxed),
        )
    }
}

impl Default for JitMetrics {
    fn default() -> Self {
        Self::new()
    }
}
