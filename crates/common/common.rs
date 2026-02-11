//! # ethrex-common
//!
//! Core types, constants, and utilities for the ethrex Ethereum client.
//!
//! This crate provides the foundational data structures used throughout ethrex,
//! including blocks, transactions, accounts, receipts, and chain configuration.
//! All other ethrex crates depend on these types.
//!
//! ## Core Types
//!
//! The [`types`] module contains all Ethereum data structures:
//!
//! - [`types::Block`], [`types::BlockHeader`], [`types::BlockBody`] - Block representation
//! - [`types::Transaction`] - All transaction types (Legacy, EIP-2930, EIP-1559, EIP-4844, EIP-7702)
//! - [`types::Account`], [`types::AccountInfo`], [`types::AccountState`] - Account state
//! - [`types::Receipt`], [`types::Log`] - Transaction receipts and logs
//! - [`types::Genesis`], [`types::ChainConfig`] - Chain configuration
//!
//! ## Re-exports
//!
//! This crate re-exports commonly used types for convenience:
//!
//! - All types from [`ethereum_types`] (Address, H256, U256, Bloom, etc.)
//! - [`Bytes`] from the `bytes` crate
//! - [`TrieLogger`] and [`TrieWitness`] from `ethrex-trie`
//!
//! ## Quick Start
//!
//! ```rust
//! use ethrex_common::{Address, H256, U256};
//! use ethrex_common::types::{Block, Transaction, AccountInfo};
//!
//! // Use Ethereum primitive types directly
//! let address = Address::zero();
//! let hash = H256::zero();
//! let value = U256::from(1_000_000_000_000_000_000u64);
//! ```
//!
//! ## Modules
//!
//! - [`types`]: Core Ethereum data structures
//! - [`constants`]: Protocol constants (gas limits, blob sizes, hash values)
//! - [`serde_utils`]: JSON serialization helpers for hex/decimal encoding
//! - [`evm`]: EVM utilities (CREATE address calculation)
//! - [`utils`]: General utilities (keccak hashing, U256 conversions)
//! - [`rkyv_utils`]: Zero-copy serialization wrappers for zkVM proving
//! - [`genesis_utils`]: Genesis JSON file utilities
//! - [`errors`]: Error types
//! - [`base64`]: RFC 4648 URL-safe base64 encoding
//! - [`fd_limit`]: File descriptor limit management
//! - [`tracing`]: Logging/tracing support
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `secp256k1` | Production ECDSA library (default) |
//! | `c-kzg` | Fast KZG via C bindings |
//! | `risc0` | RISC0 zkVM compatibility |
//! | `sp1` | Succinct SP1 zkVM compatibility |
//! | `zisk` | Polygon ZisK zkVM compatibility |
//! | `openvm` | OpenVM zkVM compatibility |
//!
//! ## Serialization
//!
//! All core types implement:
//! - `RLPEncode` / `RLPDecode` for network and storage serialization
//! - `Serialize` / `Deserialize` for JSON APIs
//! - Rkyv wrappers available in [`rkyv_utils`] for zkVM proving

pub use ethereum_types::*;
pub mod constants;
pub mod serde_utils;
pub mod types;
pub mod validation;
pub use bytes::Bytes;
pub mod base64;
pub use ethrex_trie::{TrieLogger, TrieWitness};
pub mod errors;
pub mod evm;
pub mod fd_limit;
pub mod genesis_utils;
pub mod rkyv_utils;
pub mod tracing;
pub mod utils;

pub use errors::{EcdsaError, InvalidBlockError};
pub use validation::{
    get_total_blob_gas, validate_block, validate_block_access_list_hash, validate_gas_used,
    validate_receipts_root, validate_requests_hash,
};
