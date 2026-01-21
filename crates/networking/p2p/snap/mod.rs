//! Snap Sync Protocol Implementation
//!
//! This module contains the server-side snap sync request processing.
//! The snap protocol enables fast state synchronization by requesting
//! account ranges, storage ranges, bytecodes, and trie nodes.
//!
//! ## Module Structure
//!
//! - `server`: Server-side request processing functions
//! - `constants`: Protocol constants and configuration values

pub mod constants;
mod server;

// Re-export public server functions
pub use server::{
    process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
    process_trie_nodes_request,
};

// Re-export crate-internal helper functions
pub(crate) use server::encodable_to_proof;
