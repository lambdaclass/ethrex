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

pub mod client;
pub mod constants;
pub mod error;
mod server;

use bytes::Bytes;

// Re-export public server functions
pub use server::{
    process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
    process_trie_nodes_request,
};

// Re-export error types
pub use error::{DumpError, SnapError};

// Re-export client types
pub use client::{RequestMetadata, RequestStorageTrieNodesError};

// Helper to convert proof to RLP-encodable format
#[inline]
pub(crate) fn proof_to_encodable(proof: Vec<Vec<u8>>) -> Vec<Bytes> {
    proof.into_iter().map(Bytes::from).collect()
}

// Helper to obtain proof from RLP-encodable format
#[inline]
pub(crate) fn encodable_to_proof(proof: &[Bytes]) -> Vec<Vec<u8>> {
    proof.iter().map(|bytes| bytes.to_vec()).collect()
}
