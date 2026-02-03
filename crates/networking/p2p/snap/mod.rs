//! Snap Sync Protocol Implementation
//!
//! This module contains the snap sync protocol implementation including
//! server-side request processing and client-side request methods.
//! The snap protocol enables fast state synchronization by requesting
//! account ranges, storage ranges, bytecodes, and trie nodes.
//!
//! ## Module Structure
//!
//! - `server`: Server-side request processing functions
//! - `client`: Client-side request methods for PeerHandler
//! - `constants`: Protocol constants and configuration values
//! - `error`: Unified error types for snap protocol operations

pub mod async_fs;
pub mod client;
pub mod constants;
pub mod error;
mod server;

// Re-export public server functions
pub use server::{
    process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
    process_trie_nodes_request,
};

// Re-export error types
pub use error::{DumpError, SnapError};

// Re-export client types
pub use client::{RequestMetadata, RequestStorageTrieNodesError};

// Re-export crate-internal helper functions
pub(crate) use server::encodable_to_proof;
