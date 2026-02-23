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
/// start the background compiler thread.
///
/// Call this once at application startup to enable JIT execution.
/// Without this registration, the JIT dispatch in `vm.rs` is a no-op
/// (counter increments but compiled code is never executed).
#[cfg(feature = "revmc-backend")]
pub fn register_jit_backend() {
    use ethrex_levm::jit::compiler_thread::{CompilerRequest, CompilerThread};
    use std::sync::Arc;

    let backend = Arc::new(backend::RevmcBackend::default());
    let backend_for_thread = Arc::clone(&backend);
    let cache = ethrex_levm::vm::JIT_STATE.cache.clone();

    ethrex_levm::vm::JIT_STATE.register_backend(backend);

    // Start background compiler thread that handles both Compile and Free requests
    let compiler_thread = CompilerThread::start(move |request| {
        match request {
            CompilerRequest::Compile(req) => {
                match backend_for_thread.compile(&req.code, req.fork, &cache) {
                    Ok(()) => {
                        use std::sync::atomic::Ordering;
                        ethrex_levm::vm::JIT_STATE
                            .metrics
                            .compilations
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!(
                            "[JIT] background compilation failed for {}: {e}",
                            req.code.hash
                        );
                    }
                }
            }
            CompilerRequest::Free { func_id } => {
                // LLVM function memory management.
                // Currently a no-op because we don't have a persistent LLVM context
                // that can free individual functions. The func_id is tracked for
                // metrics and future implementation.
                eprintln!("[JIT] free request for func_id={func_id} (no-op in current PoC)");
            }
        }
    });
    ethrex_levm::vm::JIT_STATE.register_compiler_thread(compiler_thread);
}

#[cfg(test)]
mod tests;
