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
//! let mut vm = VM::new(env, db, &tx, tracer, debug_mode, vm_type);
//!
//! // Execute the transaction
//! let report = vm.execute()?;
//!
//! // Check execution result
//! if report.is_success() {
//!     println!("Gas used: {}", report.gas_used);
//! }
//! ```

// =============================================================================
// Internal U256/U512 Types
// =============================================================================
//
// LEVM uses ruint internally for 256-bit arithmetic operations. This provides
// better performance for the hot paths in EVM execution (arithmetic, stack
// operations, memory access) compared to ethereum_types::U256.
//
// At boundaries where LEVM interfaces with ethrex_common (which uses
// ethereum_types::U256), we convert using from_eth_u256() and to_eth_u256().
// These conversions are essentially zero-cost since both types share the same
// memory layout (4 x u64 limbs in little-endian order).

/// Internal 256-bit unsigned integer type based on ruint.
/// Used for all EVM arithmetic and stack operations within LEVM.
pub type U256 = ruint::Uint<256, 4>;

/// Internal 512-bit unsigned integer type based on ruint.
/// Used for intermediate results in multiplication operations (e.g., MULMOD).
pub type U512 = ruint::Uint<512, 8>;

/// Type alias for ethrex_common's U256 (ethereum_types based).
/// Used at crate boundaries when interfacing with external code.
pub type EthU256 = ethrex_common::U256;

/// Convert from ethrex_common::U256 to internal ruint U256.
///
/// This is a zero-cost conversion as both types have identical memory layouts.
/// Use this when receiving U256 values from external code (e.g., transaction
/// values, block data, storage reads).
#[inline]
pub fn from_eth_u256(v: EthU256) -> U256 {
    U256::from_limbs(v.0)
}

/// Convert from internal ruint U256 to ethrex_common::U256.
///
/// This is a zero-cost conversion as both types have identical memory layouts.
/// Use this when returning U256 values to external code (e.g., storage writes,
/// balance updates, public API returns).
#[inline]
pub fn to_eth_u256(v: U256) -> EthU256 {
    ethrex_common::U256(*v.as_limbs())
}

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
