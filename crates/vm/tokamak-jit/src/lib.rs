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

// The adapter, compiler, and backend modules require revmc + revm types.
#[cfg(feature = "revmc-backend")]
pub mod adapter;
#[cfg(feature = "revmc-backend")]
pub mod backend;
#[cfg(feature = "revmc-backend")]
pub mod compiler;

// Re-exports for convenience
pub use error::JitError;
pub use ethrex_levm::jit::{
    cache::CodeCache,
    counter::ExecutionCounter,
    types::{AnalyzedBytecode, JitConfig, JitOutcome},
};

#[cfg(test)]
mod tests;
