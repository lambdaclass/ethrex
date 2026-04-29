//! Snap Sync Protocol RLPx Messages
//!
//! This module contains the message types and codec implementations for
//! the snap sync protocol (snap/1 and snap/2).
//!
//! ## Module Structure
//!
//! - `messages`: Message struct definitions
//! - `codec`: RLPxMessage and RLP encoding implementations
//!
//! ## Protocol Overview
//!
//! snap/1 defines 8 message types:
//! - GetAccountRange / AccountRange: Request/response for account state ranges
//! - GetStorageRanges / StorageRanges: Request/response for storage ranges
//! - GetByteCodes / ByteCodes: Request/response for contract bytecodes
//! - GetTrieNodes / TrieNodes: Request/response for trie nodes (snap/1 only)
//!
//! snap/2 (EIP-8189) adds:
//! - GetBlockAccessLists / BlockAccessLists: Request/response for BALs (snap/2 only)
//!
//! Note: snap/2 does NOT support GetTrieNodes / TrieNodes.

mod codec;
mod messages;

// Re-export all snap/1 message types
pub use messages::{
    AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges,
    GetTrieNodes, StorageRanges, StorageSlot, TrieNodes,
};

// Re-export snap/2 message types (EIP-8189)
pub use messages::{BlockAccessLists, GetBlockAccessLists};

// Re-export message codes for protocol handling
pub use codec::codes;
pub use codec::codes::{BLOCK_ACCESS_LISTS, GET_BLOCK_ACCESS_LISTS};
