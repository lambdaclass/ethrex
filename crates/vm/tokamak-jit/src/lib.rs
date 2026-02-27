//! Tokamak JIT Compiler — revmc/LLVM-based JIT for LEVM.
//!
//! This crate provides the heavy compilation backend for LEVM's tiered
//! JIT execution system. It wraps [revmc](https://github.com/paradigmxyz/revmc)
//! (Paradigm's EVM JIT compiler) and bridges LEVM's type system to
//! revm's types that revmc expects.
//!
//! # Architecture
//!
//! ```text
//! ethrex-levm (lightweight JIT infra)
//!   └── jit/cache, jit/counter, jit/dispatch
//!
//! tokamak-jit (this crate — heavy deps)
//!   ├── adapter   — LEVM ↔ revm type conversion
//!   ├── compiler  — revmc/LLVM wrapper
//!   ├── backend   — high-level compile & cache API
//!   └── validation — dual-execution correctness checks
//! ```
//!
//! # Feature Flags
//!
//! - `revmc-backend`: Enables the revmc/LLVM compilation backend.
//!   Requires LLVM 21 installed on the system. Without this feature,
//!   only the adapter utilities and validation logic are available.

pub mod error;
pub mod validation;

// The adapter, compiler, backend, host, and execution modules require revmc + revm types.
#[cfg(feature = "revmc-backend")]
pub mod adapter;
#[cfg(feature = "revmc-backend")]
pub mod backend;
#[cfg(feature = "revmc-backend")]
pub mod compiler;
#[cfg(feature = "revmc-backend")]
pub mod execution;
#[cfg(feature = "revmc-backend")]
pub mod host;

// Re-exports for convenience
pub use error::JitError;
pub use ethrex_levm::jit::{
    cache::CodeCache,
    counter::ExecutionCounter,
    types::{AnalyzedBytecode, JitConfig, JitOutcome},
};

/// Register the revmc JIT backend with LEVM's global JIT state and
/// start the background compiler thread pool.
///
/// Call this once at application startup to enable JIT execution.
/// Without this registration, the JIT dispatch in `vm.rs` is a no-op
/// (counter increments but compiled code is never executed).
///
/// The compiler pool spawns `compile_workers` threads (default: `num_cpus / 2`),
/// each with its own thread-local `ArenaState` for LLVM context safety.
/// When all functions in an arena are evicted from the cache, the arena
/// (and its LLVM resources) is freed on the worker thread that owns it.
#[cfg(feature = "revmc-backend")]
pub fn register_jit_backend() {
    use ethrex_levm::jit::analyzer::analyze_bytecode;
    use ethrex_levm::jit::compiler_thread::{CompilerRequest, CompilerThreadPool};
    use ethrex_levm::jit::dispatch::JitBackend;
    use ethrex_levm::jit::optimizer;
    use std::collections::HashMap;
    use std::sync::Arc;

    let backend = Arc::new(backend::RevmcBackend::default());
    let cache = ethrex_levm::vm::JIT_STATE.cache.clone();
    let arena_capacity = ethrex_levm::vm::JIT_STATE.config.arena_capacity;
    let num_workers = num_cpus::get().max(2) / 2;

    ethrex_levm::vm::JIT_STATE.register_backend(backend);

    // Start background compiler pool with arena-managed compilation.
    // Each worker has its own thread-local ArenaState — ArenaCompilers are
    // only created and dropped on their owning worker thread, satisfying
    // the LLVM context thread-affinity invariant.
    let compiler_pool = CompilerThreadPool::start(num_workers, move |request| {
        // Thread-local arena state — persists across requests on this worker.
        // Using thread_local! to avoid capturing mutable state in Fn closure.
        thread_local! {
            static ARENA_STATE: std::cell::RefCell<ArenaState> =
                std::cell::RefCell::new(ArenaState::new());
        }

        match request {
            CompilerRequest::Compile(req) => {
                use std::sync::atomic::Ordering;

                let cache_key = (req.code.hash, req.fork);

                // Early size check
                if ethrex_levm::vm::JIT_STATE
                    .config
                    .is_bytecode_oversized(req.code.bytecode.len())
                {
                    ethrex_levm::vm::JIT_STATE.mark_oversized(req.code.hash);
                    ethrex_levm::vm::JIT_STATE
                        .metrics
                        .compilation_skips
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }

                // Skip empty bytecodes
                if req.code.bytecode.is_empty() {
                    return;
                }

                // Deduplication guard: skip if another worker is already compiling this
                if !ethrex_levm::vm::JIT_STATE.try_start_compilation(cache_key) {
                    return;
                }

                // Drop guard ensures finish_compilation runs even if we panic
                // during analyze/optimize/compile. Without this, a panic would
                // permanently lock this key in compiling_in_progress.
                struct CompilationGuard {
                    key: ethrex_levm::jit::cache::CacheKey,
                }
                impl Drop for CompilationGuard {
                    fn drop(&mut self) {
                        ethrex_levm::vm::JIT_STATE.finish_compilation(&self.key);
                    }
                }
                let _guard = CompilationGuard { key: cache_key };

                // Analyze + optimize bytecode
                let analyzed = analyze_bytecode(
                    req.code.bytecode.clone(),
                    req.code.hash,
                    req.code.jump_targets.clone(),
                );
                let (analyzed, opt_stats) = optimizer::optimize(analyzed);
                if opt_stats.patterns_folded > 0 {
                    tracing::info!(
                        hash = %req.code.hash,
                        patterns_folded = opt_stats.patterns_folded,
                        opcodes_eliminated = opt_stats.opcodes_eliminated,
                        "Bytecode optimized before JIT compilation"
                    );
                }

                if analyzed.has_external_calls {
                    tracing::info!(
                        hash = %req.code.hash,
                        "JIT compiling bytecode with external calls"
                    );
                }

                // Compile in arena
                ARENA_STATE.with(|state| {
                    let mut state = state.borrow_mut();

                    // Get or create current arena
                    let arena = state.get_or_create_arena(
                        arena_capacity,
                        &ethrex_levm::vm::JIT_STATE.arena_manager,
                    );

                    match compiler::TokamakCompiler::compile_in_arena(arena, &analyzed, req.fork) {
                        Ok(compiled) => {
                            // Insert into cache — may trigger eviction
                            let evicted_slot = cache.insert(cache_key, compiled);

                            // Handle eviction: mark slot in arena manager
                            if let Some(slot) = evicted_slot {
                                let arena_empty =
                                    ethrex_levm::vm::JIT_STATE.arena_manager.mark_evicted(slot);
                                if arena_empty {
                                    // Drop the ArenaCompiler to free LLVM resources
                                    state.free_arena(slot.0);
                                    ethrex_levm::vm::JIT_STATE
                                        .arena_manager
                                        .remove_arena(slot.0);
                                    ethrex_levm::vm::JIT_STATE
                                        .metrics
                                        .arenas_freed
                                        .fetch_add(1, Ordering::Relaxed);
                                }
                                ethrex_levm::vm::JIT_STATE
                                    .metrics
                                    .functions_evicted
                                    .fetch_add(1, Ordering::Relaxed);
                            }

                            ethrex_levm::vm::JIT_STATE
                                .metrics
                                .compilations
                                .fetch_add(1, Ordering::Relaxed);

                            tracing::info!(
                                hash = %req.code.hash,
                                fork = ?req.fork,
                                bytecode_size = req.code.bytecode.len(),
                                basic_blocks = analyzed.basic_blocks.len(),
                                "JIT compiled bytecode"
                            );
                        }
                        Err(e) => {
                            eprintln!(
                                "[JIT] background compilation failed for {}: {e}",
                                req.code.hash
                            );
                        }
                    }
                });

                // _guard dropped here — finish_compilation called automatically
            }
            CompilerRequest::Free { slot } => {
                // Mark the function as evicted in the arena manager
                let arena_empty = ethrex_levm::vm::JIT_STATE.arena_manager.mark_evicted(slot);
                if arena_empty {
                    ARENA_STATE.with(|state| {
                        state.borrow_mut().free_arena(slot.0);
                    });
                    ethrex_levm::vm::JIT_STATE
                        .arena_manager
                        .remove_arena(slot.0);
                    ethrex_levm::vm::JIT_STATE
                        .metrics
                        .arenas_freed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                ethrex_levm::vm::JIT_STATE
                    .metrics
                    .functions_evicted
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            CompilerRequest::FreeArena { arena_id } => {
                // Force-free an entire arena
                ARENA_STATE.with(|state| {
                    state.borrow_mut().free_arena(arena_id);
                });
                ethrex_levm::vm::JIT_STATE
                    .arena_manager
                    .remove_arena(arena_id);
                ethrex_levm::vm::JIT_STATE
                    .metrics
                    .arenas_freed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });
    ethrex_levm::vm::JIT_STATE.register_compiler_pool(compiler_pool);
}

/// Thread-local state for each background compiler worker.
///
/// Manages `ArenaCompiler` instances, rotating to a new arena when
/// the current one is full. Each worker thread gets its own instance
/// via `thread_local!`, ensuring LLVM context thread-affinity.
#[cfg(feature = "revmc-backend")]
struct ArenaState {
    /// Active arenas indexed by ID. Dropped arenas free LLVM resources.
    arena_compilers: HashMap<ethrex_levm::jit::arena::ArenaId, compiler::ArenaCompiler>,
    /// ID of the current (not-yet-full) arena.
    current_arena_id: Option<ethrex_levm::jit::arena::ArenaId>,
}

#[cfg(feature = "revmc-backend")]
impl ArenaState {
    fn new() -> Self {
        Self {
            arena_compilers: HashMap::new(),
            current_arena_id: None,
        }
    }

    /// Get the current arena or create a new one if needed.
    fn get_or_create_arena(
        &mut self,
        capacity: u16,
        arena_manager: &ethrex_levm::jit::arena::ArenaManager,
    ) -> &mut compiler::ArenaCompiler {
        // Check if current arena exists and has space
        let need_new = match self.current_arena_id {
            Some(id) => self.arena_compilers.get(&id).map_or(true, |a| a.is_full()),
            None => true,
        };

        if need_new {
            let arena_id = arena_manager.allocate_arena(capacity);
            let arena = compiler::ArenaCompiler::new(arena_id, capacity);
            self.arena_compilers.insert(arena_id, arena);
            self.current_arena_id = Some(arena_id);
            ethrex_levm::vm::JIT_STATE
                .metrics
                .arenas_created
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        let id = self
            .current_arena_id
            .expect("arena must exist after creation");
        self.arena_compilers
            .get_mut(&id)
            .expect("arena must be in map after insertion")
    }

    /// Drop an arena compiler, freeing its LLVM resources.
    fn free_arena(&mut self, arena_id: ethrex_levm::jit::arena::ArenaId) {
        if self.arena_compilers.remove(&arena_id).is_some() {
            tracing::info!(arena_id, "Freed arena LLVM resources");
        }
        // If we freed the current arena, clear the reference
        if self.current_arena_id == Some(arena_id) {
            self.current_arena_id = None;
        }
    }
}

#[cfg(test)]
mod tests;
