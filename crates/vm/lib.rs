//! # ethrex-vm
//!
//! High-level EVM execution layer for the ethrex Ethereum client.
//!
//! ## Overview
//!
//! This crate provides a high-level abstraction over the LEVM (Lambda EVM)
//! execution engine. It wraps LEVM with additional functionality for:
//!
//! - Block and transaction execution
//! - State management via [`VmDatabase`] trait
//! - Witness generation for zkVM proving
//! - System contract handling (EIP-7002, EIP-7251)
//!
//! ## Quick Start
//!
//! ```ignore
//! use ethrex_vm::{Evm, BlockExecutionResult, ExecutionResult};
//!
//! // Create EVM with database
//! let evm = Evm::new_for_l1(db)?;
//!
//! // Execute a full block
//! let result: BlockExecutionResult = evm.execute_block(&block, &header)?;
//!
//! // Or simulate a transaction
//! let result: ExecutionResult = evm.simulate_tx_from_generic(&tx, &header)?;
//! ```
//!
//! ## Core Types
//!
//! - [`Evm`]: Main execution engine wrapping LEVM
//! - [`VmDatabase`]: Trait for state access (account state, storage, code)
//! - [`ExecutionResult`]: Transaction execution outcome (Success/Revert/Halt)
//! - [`BlockExecutionResult`]: Block execution result with receipts and requests
//! - [`GuestProgramStateWrapper`]: Thread-safe wrapper for zkVM witness state
//!
//! ## Modules
//!
//! - [`backends`]: EVM backend implementations (LEVM wrapper)
//! - [`system_contracts`]: System contract addresses by fork
//! - [`tracing`]: Call tracing support
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `secp256k1` | Production ECDSA library (default) |
//! | `c-kzg` | C KZG implementation for EIP-4844 |
//! | `sp1` | Succinct SP1 zkVM support |
//! | `risc0` | RISC Zero zkVM support |
//! | `zisk` | Polygon ZisK zkVM support |
//! | `openvm` | OpenVM zkVM support |

mod db;
mod errors;
mod execution_result;
pub mod tracing;
mod witness_db;

pub mod backends;

pub use backends::{BlockExecutionResult, Evm};
pub use db::{DynVmDatabase, VmDatabase};
pub use errors::EvmError;
pub use ethrex_levm::precompiles::{PrecompileCache, precompiles_for_fork};
pub use execution_result::ExecutionResult;
pub use witness_db::GuestProgramStateWrapper;
pub mod system_contracts;
