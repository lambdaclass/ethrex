//! # LEVM - Lambda EVM
//!
//! A pure Rust implementation of the Ethereum Virtual Machine.
//!
//! ## Overview
//!
//! LEVM (Lambda EVM) is ethrex's native EVM implementation, designed for:
//! - **Correctness**: Full compatibility with Ethereum consensus tests
//! - **Performance**: Optimized opcode execution and memory management
//! - **Readability**: Clean, well-documented Rust code
//! - **Extensibility**: Modular design for easy feature additions
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                           VM                                 │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
//! │  │  CallFrame  │  │   Memory    │  │       Stack         │ │
//! │  └─────────────┘  └─────────────┘  └─────────────────────┘ │
//! │                                                             │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
//! │  │  Substate   │  │ Precompiles │  │   Environment       │ │
//! │  └─────────────┘  └─────────────┘  └─────────────────────┘ │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    GeneralizedDatabase                       │
//! │              (Account state, storage, code)                  │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Key Components
//!
//! - [`vm::VM`]: Main EVM execution engine
//! - [`call_frame::CallFrame`]: Execution context for each call
//! - [`memory::Memory`]: EVM memory with expansion tracking
//! - [`environment::Environment`]: Block and transaction context
//! - [`precompiles`]: Native implementations of precompiled contracts
//! - [`hooks`]: Execution hooks for pre/post-execution logic and L2-specific behavior
//!
//! ## Supported Forks
//!
//! LEVM supports post-merge Ethereum forks:
//! - Paris (The Merge), Shanghai, Cancun, Prague, Osaka
//!
//! Note: ethrex is a post-merge client and does not support pre-merge forks.
//!
//! ## Usage
//!
//! ```ignore
//! use levm::{VM, Environment};
//!
//! // Create VM with database and environment
//! let mut vm = VM::new(env, db, &tx, tracer, vm_type, &NativeCrypto);
//!
//! // Execute the transaction
//! let report = vm.execute()?;
//!
//! // Check execution result
//! if report.is_success() {
//!     println!("Gas used: {}", report.gas_used);
//! }
//! ```

pub mod call_frame;
pub mod constants;
pub mod db;
pub mod debug;
pub mod environment;
pub mod errors;
pub mod execution_handlers;
pub mod gas_cost;
pub mod hooks;
/// Host-only result cache for the `KECCAK256` opcode. Compiled out of every zkVM
/// guest (all of which target riscv32 — sp1/risc0/openvm — or riscv64 — zisk),
/// which must keep the opcode on its direct, provable path.
#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
pub mod keccak_cache;
pub mod memory;
pub mod opcode_handlers;
pub mod opcode_tracer;
pub mod opcodes;
pub mod precompiles;
pub mod tracing;
pub mod utils;
pub mod vm;
pub use environment::*;
pub mod account;
#[cfg(feature = "perf_opcode_timings")]
pub mod timings;
