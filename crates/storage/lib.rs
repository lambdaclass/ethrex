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
//! State is now managed using the high-performance `ethrex_db` storage engine:
//! - **Hot Storage**: Recent unfinalized blocks in the Blockchain layer (COW semantics)
//! - **Cold Storage**: Finalized blocks in PagedDb (memory-mapped pages)
//! - **Performance**: 10-15x faster reads, 1.6-2.2x faster writes, 12-13x faster state roots
//!
//! ### Legacy Trie Layer (Deprecated)
//!
//! The old trie implementation (`ethrex-trie`, `TrieLayerCache`) is deprecated and
//! will be removed in a future release. New code should use the `*_ethrex_db()` methods
//! on the Store struct.

pub mod api;
pub mod backend;
pub mod error;
mod ethrex_db_adapter;
pub mod rlp;
pub mod store;
pub mod utils;

// Legacy modules - deprecated, will be removed
#[deprecated(since = "9.1.0", note = "Use ethrex_db storage methods instead")]
mod layering;
#[deprecated(since = "9.1.0", note = "Use ethrex_db storage methods instead")]
pub mod trie;

// Legacy exports - deprecated
#[deprecated(since = "9.1.0", note = "Use ethrex_db storage methods instead")]
pub use layering::apply_prefix;

pub use store::{AccountUpdatesList, EngineType, Store, UpdateBatch, hash_address, hash_key};

/// Store Schema Version, must be updated on any breaking change.
///
/// An upgrade to a newer schema version invalidates currently stored data,
/// requiring a re-sync from genesis or a snapshot.
///
/// Version 2: Migrated from ethrex-trie to ethrex_db storage engine.
/// This is a breaking change that requires a full resync from genesis.
pub const STORE_SCHEMA_VERSION: u64 = 2;

/// Name of the file storing the metadata about the database.
///
/// This file contains version information and is used to detect
/// incompatible database formats on startup.
pub const STORE_METADATA_FILENAME: &str = "metadata.json";
