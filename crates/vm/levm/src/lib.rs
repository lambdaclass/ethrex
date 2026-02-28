//! # ethrex-levm - Lambda EVM
//!
//! A pure Rust implementation of the Ethereum Virtual Machine.
//!
//! ## Overview
//!
//! LEVM (Lambda EVM) is ethrex's native EVM implementation, designed for:
//! - **Correctness**: Full compatibility with Ethereum consensus tests
//! - **Performance**: Optimized opcode execution and memory management
//! - **Readability**: Clean, well-documented Rust code
//! - **Extensibility**: Modular design with hooks for L1/L2 customization
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
//! ## Core Types
//!
//! - [`vm::VM`]: Main EVM execution engine
//! - [`call_frame::CallFrame`]: Execution context for each call
//! - [`memory::Memory`]: EVM memory with expansion tracking
//! - [`environment::Environment`]: Block and transaction context
//! - [`precompiles`]: Native implementations of precompiled contracts (17 total)
//! - [`hooks`]: Execution hooks for L1/L2-specific behavior
//! - [`db`]: Database trait and [`db::GeneralizedDatabase`] wrapper
//!
//! ## Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`vm`] | Main VM execution engine |
//! | [`call_frame`] | CallFrame and Stack types |
//! | [`memory`] | EVM memory with expansion tracking |
//! | [`environment`] | Block and transaction context |
//! | [`opcodes`] | Opcode enum (179 opcodes) |
//! | [`opcode_handlers`] | Opcode execution logic by category |
//! | [`precompiles`] | Native precompiled contracts |
//! | [`hooks`] | L1/L2 execution hooks |
//! | [`db`] | Database trait and GeneralizedDatabase |
//! | [`errors`] | VMError, ExceptionalHalt, etc. |
//! | [`gas_cost`] | Gas cost calculations |
//! | [`tracing`] | Geth-compatible call tracer |
//!
//! ## Supported Forks
//!
//! LEVM supports post-merge Ethereum forks:
//! - Paris (The Merge), Shanghai, Cancun, Prague, Osaka
//!
//! Note: ethrex is a post-merge client and does not support pre-merge forks.
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `secp256k1` | Production ECDSA library (default) |
//! | `c-kzg` | C KZG implementation |
//! | `sp1` | Succinct SP1 zkVM support |
//! | `risc0` | RISC Zero zkVM support |
//! | `zisk` | Polygon ZisK zkVM support |
//! | `openvm` | OpenVM zkVM support |
//!
//! ## Quick Start
//!
//! ```ignore
//! use ethrex_levm::{vm::VM, Environment, VMType};
//! use ethrex_levm::db::GeneralizedDatabase;
//!
//! // Create VM with database and environment
//! let mut vm = VM::new(env, &mut db, &tx, tracer, debug_mode, vm_type)?;
//!
//! // Execute the transaction
//! let report = vm.execute()?;
//!
//! // Check execution result
//! if report.result.is_success() {
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
pub mod memory;
pub mod opcode_handlers;
pub mod opcodes;
pub mod precompiles;
pub mod tracing;
pub mod utils;
pub mod vm;
pub use environment::*;
pub mod account;
#[cfg(feature = "perf_opcode_timings")]
pub mod timings;
