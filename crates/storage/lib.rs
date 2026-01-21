//! # ethrex-storage
//!
//! Persistent storage layer for the ethrex Ethereum client.
//!
//! ## Overview
//!
//! This crate provides a high-level storage API ([`Store`]) for blockchain operations,
//! abstracting away pluggable storage backends. It handles:
//!
//! - Block storage (headers, bodies, receipts)
//! - State storage (accounts, code, storage slots)
//! - Merkle Patricia Trie management
//! - Transaction indexing
//! - Chain configuration
//! - Execution witnesses for zkVM proving
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
//! ## Quick Start
//!
//! ```ignore
//! use ethrex_storage::{Store, EngineType};
//! use std::path::Path;
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
//! let info = store.get_account_info(block_number, address).await?;
//! let balance = info.map(|a| a.balance);
//!
//! // Get storage value
//! let value = store.get_storage_at(block_hash, address, key)?;
//! ```
//!
//! ## Modules
//!
//! - [`store`]: Main [`Store`] type with blockchain operations
//! - [`api`]: [`api::StorageBackend`] trait and table definitions
//! - [`backend`]: Backend implementations (InMemory, RocksDB)
//! - [`trie`]: Trie database adapters
//! - [`error`]: [`error::StoreError`] type
//!
//! ## State Management
//!
//! State is stored using Merkle Patricia Tries for efficient verification:
//! - **State Trie**: Maps account addresses to account data
//! - **Storage Tries**: Maps storage keys to values for each contract
//! - **Code Storage**: Separate storage for contract bytecode
//!
//! The store maintains a cache layer for efficient state access without
//! requiring full trie traversal for recent blocks.
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `rocksdb` | Enable RocksDB backend for persistent storage |

pub mod api;
pub mod backend;
pub mod error;
mod layering;
pub mod rlp;
pub mod store;
pub mod trie;
pub mod utils;

pub use layering::apply_prefix;
pub use store::{AccountUpdatesList, EngineType, Store, UpdateBatch, hash_address, hash_key};

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
