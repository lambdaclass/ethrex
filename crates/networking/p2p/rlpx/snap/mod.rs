//! Snap Sync Protocol RLPx Messages
//!
//! This module contains the message types and codec implementations for
//! the snap sync protocol (snap/1).
//!
//! ## Module Structure
//!
//! - `messages`: Message struct definitions
//! - `codec`: RLPxMessage and RLP encoding implementations
//!
//! ## Protocol Overview
//!
//! The snap protocol defines 8 message types:
//! - GetAccountRange / AccountRange: Request/response for account state ranges
//! - GetStorageRanges / StorageRanges: Request/response for storage ranges
//! - GetByteCodes / ByteCodes: Request/response for contract bytecodes
//! - GetTrieNodes / TrieNodes: Request/response for trie nodes

mod codec;
mod messages;

// Re-export all message types
pub use messages::{
    AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges,
    GetTrieNodes, StorageRanges, StorageSlot, TrieNodes,
};

// Re-export message codes for protocol handling
pub use codec::codes;
