//! revmc/LLVM compiler wrapper.
//!
//! Wraps the revmc `EvmCompiler` + `EvmLlvmBackend` pipeline, providing
//! a simplified API for compiling EVM bytecode to native code.
//!
//! Two compilation paths are provided:
//!
//! - [`TokamakCompiler::compile`]: Standalone compilation that leaks the LLVM
//!   compiler (suitable for testing and the initial PoC).
//! - [`TokamakCompiler::compile_in_arena`]: Arena-managed compilation that stores
//!   the LLVM compiler in an [`ArenaCompiler`], enabling proper memory lifecycle
//!   management. When the arena is dropped, all its LLVM resources are freed.

use crate::adapter::fork_to_spec_id;
use crate::error::JitError;
use ethrex_common::types::Fork;
use ethrex_levm::jit::arena::ArenaId;
use ethrex_levm::jit::cache::CompiledCode;
use ethrex_levm::jit::types::AnalyzedBytecode;

use revmc::{EvmCompiler, EvmLlvmBackend, OptimizationLevel};
use revmc_context::EvmCompilerFn;

/// Owns one or more LLVM compilers that have produced JIT-compiled functions.
///
/// Each compiler contains an LLVM execution engine that owns the JIT code
/// memory. Storing compilers here (instead of leaking them via `mem::forget`)
/// lets us free LLVM resources when the arena is no longer needed.
///
/// # Safety invariant
///
/// The compilers stored here were created using `revmc::llvm::with_llvm_context`,
/// which provides a thread-local LLVM context. The context persists for the
/// lifetime of the thread. `ArenaCompiler` must only be dropped on the same
/// thread that created its compilers — specifically, the background compiler
/// thread managed by `CompilerThreadPool`.
///
/// The caller must ensure that no function pointers from this arena are in
/// active use when the `ArenaCompiler` is dropped. The `ArenaManager` tracks
/// live function counts to enforce this.
pub struct ArenaCompiler {
    arena_id: ArenaId,
    /// Type-erased compilers. Each `Box<dyn Any + Send>` is an
    /// `EvmCompiler<EvmLlvmBackend<'_>>` that owns its LLVM execution engine
    /// and the JIT code memory for one compiled function. Dropping these
    /// frees the JIT code.
    compilers: Vec<Box<dyn std::any::Any + Send>>,
    compiled_count: u16,
    capacity: u16,
}

impl ArenaCompiler {
    /// Create a new arena compiler with the given ID and capacity.
    ///
    /// `capacity` is the maximum number of functions this arena can hold.
    /// Once full, a new arena must be allocated via `ArenaManager`.
    pub fn new(arena_id: ArenaId, capacity: u16) -> Self {
        Self {
            arena_id,
            compilers: Vec::with_capacity(usize::from(capacity)),
            compiled_count: 0,
            capacity,
        }
    }

    /// The arena ID assigned by `ArenaManager`.
    pub fn arena_id(&self) -> ArenaId {
        self.arena_id
    }

    /// Whether this arena has reached its function capacity.
    pub fn is_full(&self) -> bool {
        self.compiled_count >= self.capacity
    }

    /// Number of functions compiled into this arena so far.
    pub fn compiled_count(&self) -> u16 {
        self.compiled_count
    }

    /// Maximum number of functions this arena can hold.
    pub fn capacity(&self) -> u16 {
        self.capacity
    }
}

impl std::fmt::Debug for ArenaCompiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArenaCompiler")
            .field("arena_id", &self.arena_id)
            .field("compiled_count", &self.compiled_count)
            .field("capacity", &self.capacity)
            .finish()
    }
}

/// JIT compiler backed by revmc + LLVM.
///
/// Provides static methods for compiling EVM bytecode to native code.
/// Compiled function pointers are returned as `CompiledCode` for insertion
/// into the global `CodeCache`.
pub struct TokamakCompiler {
    _marker: std::marker::PhantomData<()>,
}

impl TokamakCompiler {
    /// Compile analyzed bytecode into native code for a specific fork.
    ///
    /// This is the **standalone** compilation path that leaks the LLVM compiler
    /// via `mem::forget`. The JIT code memory is never freed. Suitable for
    /// testing and the initial PoC, but should not be used in production.
    ///
    /// For arena-managed compilation with proper cleanup, use
    /// [`compile_in_arena`](Self::compile_in_arena) instead.
    pub fn compile(analyzed: &AnalyzedBytecode, fork: Fork) -> Result<CompiledCode, JitError> {
        let bytecode = analyzed.bytecode.as_ref();
        let hash_hex = format!("{:x}", analyzed.hash);
        let spec_id = fork_to_spec_id(fork);

        revmc::llvm::with_llvm_context(|cx| {
            let backend = EvmLlvmBackend::new(cx, false, OptimizationLevel::Aggressive)
                .map_err(|e| JitError::LlvmError(format!("backend init: {e}")))?;

            let mut compiler = EvmCompiler::new(backend);

            #[expect(unsafe_code)]
            let f: EvmCompilerFn = unsafe {
                compiler
                    .jit(&hash_hex, bytecode, spec_id)
                    .map_err(|e| JitError::CompilationFailed(format!("{e}")))?
            };

            let raw_fn = f.into_inner();

            #[expect(unsafe_code, clippy::as_conversions)]
            let compiled = unsafe {
                CompiledCode::new(
                    raw_fn as *const (),
                    analyzed.bytecode.len(),
                    analyzed.basic_blocks.len(),
                    None, // No arena — standalone leak path
                    analyzed.has_external_calls,
                )
            };

            // Leak the compiler so JIT code memory outlives this closure.
            // This is intentional for the standalone path. See compile_in_arena
            // for the arena-managed alternative.
            std::mem::forget(compiler);

            Ok(compiled)
        })
    }

    /// Compile analyzed bytecode and store the compiler in an arena.
    ///
    /// Instead of leaking the compiler via `mem::forget`, the compiler is
    /// pushed into the arena's storage. When the arena is dropped, all its
    /// compilers are dropped, freeing the LLVM JIT code memory.
    ///
    /// The returned `CompiledCode` carries an `arena_slot` field linking it
    /// back to this arena, so cache eviction can decrement the arena's live
    /// function count via `ArenaManager::mark_evicted`.
    pub fn compile_in_arena(
        arena: &mut ArenaCompiler,
        analyzed: &AnalyzedBytecode,
        fork: Fork,
    ) -> Result<CompiledCode, JitError> {
        if arena.is_full() {
            return Err(JitError::CompilationFailed(format!(
                "arena {} is full ({}/{})",
                arena.arena_id, arena.compiled_count, arena.capacity
            )));
        }

        let bytecode = analyzed.bytecode.as_ref();
        let hash_hex = format!("{:x}", analyzed.hash);
        let spec_id = fork_to_spec_id(fork);
        let slot_idx = arena.compiled_count;
        let arena_id = arena.arena_id;

        revmc::llvm::with_llvm_context(|cx| {
            let backend = EvmLlvmBackend::new(cx, false, OptimizationLevel::Aggressive)
                .map_err(|e| JitError::LlvmError(format!("backend init: {e}")))?;

            let mut compiler = EvmCompiler::new(backend);

            #[expect(unsafe_code)]
            let f: EvmCompilerFn = unsafe {
                compiler
                    .jit(&hash_hex, bytecode, spec_id)
                    .map_err(|e| JitError::CompilationFailed(format!("{e}")))?
            };

            let raw_fn = f.into_inner();

            #[expect(unsafe_code, clippy::as_conversions)]
            let compiled = unsafe {
                CompiledCode::new(
                    raw_fn as *const (),
                    analyzed.bytecode.len(),
                    analyzed.basic_blocks.len(),
                    Some((arena_id, slot_idx)),
                    analyzed.has_external_calls,
                )
            };

            // Store compiler in arena instead of leaking it.
            // The compiler owns the LLVM execution engine which owns the JIT
            // code. Dropping the arena drops all compilers, freeing JIT memory.
            arena.compilers.push(Box::new(compiler));
            arena.compiled_count += 1;

            Ok(compiled)
        })
    }
}

impl std::fmt::Debug for TokamakCompiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokamakCompiler").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_compiler_new() {
        let arena = ArenaCompiler::new(42, 64);
        assert_eq!(arena.arena_id(), 42);
        assert_eq!(arena.compiled_count(), 0);
        assert_eq!(arena.capacity(), 64);
        assert!(!arena.is_full());
    }

    #[test]
    fn test_arena_compiler_is_full() {
        let mut arena = ArenaCompiler::new(0, 2);
        assert!(!arena.is_full());

        arena.compiled_count = 1;
        assert!(!arena.is_full());

        arena.compiled_count = 2;
        assert!(arena.is_full());
    }

    #[test]
    fn test_arena_compiler_zero_capacity() {
        let arena = ArenaCompiler::new(0, 0);
        assert!(arena.is_full());
        assert_eq!(arena.compiled_count(), 0);
    }

    #[test]
    fn test_arena_compiler_debug() {
        let arena = ArenaCompiler::new(7, 32);
        let debug = format!("{arena:?}");
        assert!(debug.contains("ArenaCompiler"));
        assert!(debug.contains("arena_id: 7"));
        assert!(debug.contains("capacity: 32"));
    }
}
