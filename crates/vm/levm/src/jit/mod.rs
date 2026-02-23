//! JIT compilation infrastructure for LEVM.
//!
//! This module provides the lightweight in-process infrastructure for
//! tiered JIT compilation: execution counting, bytecode analysis,
//! compiled code caching, and dispatch logic.
//!
//! The actual compilation backend (revmc + LLVM) lives in the separate
//! `tokamak-jit` crate to keep LEVM free of heavy dependencies.

pub mod analyzer;
pub mod cache;
pub mod compiler_thread;
pub mod counter;
pub mod dispatch;
pub mod types;
