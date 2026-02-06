//! # ethrex Storage
//!
//! This crate provides persistent storage for the ethrex Ethereum client.
//!
//! ## Overview
//!
//! The storage layer handles:
//! - Block storage (headers, bodies, receipts)
//! - State storage (accounts, code, storage slots)
//! - Merkle Patricia Trie management
//! - Transaction indexing
//! - Chain configuration
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │                    Store                        │
//! │  (High-level API for blockchain operations)     │
//! └─────────────────────────────────────────────────┘
//!                        │
//!           ┌────────────┴────────────┐
//!           ▼                         ▼
//! ┌─────────────────┐       ┌─────────────────┐
//! │  InMemoryBackend │       │  RocksDBBackend │
//! │    (Testing)     │       │  (Production)   │
//! └─────────────────┘       └─────────────────┘
//! ```
//!
//! ## Storage Backends
//!
//! - **InMemory**: Fast, non-persistent storage for testing
//! - **RocksDB**: Production-grade persistent storage (requires `rocksdb` feature)
//!
//! ## Usage
//!
//! ```ignore
//! use ethrex_storage::{Store, EngineType};
//!
//! // Create a new store with RocksDB backend
//! let store = Store::new("./data", EngineType::RocksDB)?;
//!
//! // Or from a genesis file
//! let store = Store::new_from_genesis(
//!     Path::new("./data"),
//!     EngineType::RocksDB,
//!     "genesis.json"
//! ).await?;
//!
//! // Add a block
//! store.add_block(block).await?;
//!
//! // Query state
//! let balance = store.get_account_info(block_number, address)?.map(|a| a.balance);
//! ```
//!
//! ## State Management
//!
//! State is stored using Merkle Patricia Tries for efficient verification:
//! - **State Trie**: Maps account addresses to account data
//! - **Storage Tries**: Maps storage keys to values for each contract
//! - **Code Storage**: Separate storage for contract bytecode
//!
//! The store maintains a cache layer (`TrieLayerCache`) for efficient state access
//! without requiring full trie traversal for recent blocks.

pub mod api;
pub mod backend;
pub mod error;
mod layering;
pub mod rlp;
pub mod store;
pub mod trie;
pub mod utils;

pub use layering::apply_prefix;
pub use store::{
    AccountUpdatesList, EngineType, Store, TrieSnapshot, UpdateBatch, hash_address, hash_key,
};

/// Store Schema Version, must be updated on any breaking change.
///
/// An upgrade to a newer schema version invalidates currently stored data,
/// requiring a re-sync from genesis or a snapshot.
pub const STORE_SCHEMA_VERSION: u64 = 1;

/// Name of the file storing the metadata about the database.
///
/// This file contains version information and is used to detect
/// incompatible database formats on startup.
pub const STORE_METADATA_FILENAME: &str = "metadata.json";
