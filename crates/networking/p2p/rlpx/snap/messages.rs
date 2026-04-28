//! Snap protocol message definitions
//!
//! This module contains the message types used in the snap sync protocol.
//! Each message type implements RLPxMessage for encoding/decoding.
//!
//! # snap/2 wire layout decision (Task 2.0 — EIP-8189 §"BlockAccessLists")
//!
//! EIP-8189 does not mandate a specific encoding for unavailable (pruned or
//! unknown) BAL slots in the response. The plan default is adopted:
//!
//!   `BlockAccessLists.bals: Vec<Option<BlockAccessList>>`
//!
//! Each slot corresponds positionally to the matching entry in the request's
//! `block_hashes`. A `None` slot means the BAL is unavailable (block unknown
//! or pruned). This preserves position correspondence and simplifies callers
//! (they do not need to maintain separate "which slots were filled" metadata).
//!
//! On the wire, `None` is encoded as an RLP empty byte string (`0x80`) and
//! `Some(bal)` is encoded as the RLP-encoded `BlockAccessList` list.
//! Citation: EIP-8189 §"BlockAccessLists" response, implementation-defined
//! for unavailable entries.

use bytes::Bytes;
use ethrex_common::{
    H256, U256,
    types::{AccountState, block_access_list::BlockAccessList},
};

// =============================================================================
// REQUEST MESSAGES
// =============================================================================

/// Request a range of accounts from the state trie.
#[derive(Debug, Clone)]
pub struct GetAccountRange {
    /// Request ID - the responding peer must mirror this value
    pub id: u64,
    /// State root hash to query against
    pub root_hash: H256,
    /// Starting hash of the account range
    pub starting_hash: H256,
    /// Limit hash of the account range (inclusive)
    pub limit_hash: H256,
    /// Maximum response size in bytes
    pub response_bytes: u64,
}

/// Request storage ranges for multiple accounts.
#[derive(Debug, Clone)]
pub struct GetStorageRanges {
    /// Request ID - the responding peer must mirror this value
    pub id: u64,
    /// State root hash to query against
    pub root_hash: H256,
    /// List of account hashes to get storage for
    pub account_hashes: Vec<H256>,
    /// Starting hash of the storage range
    pub starting_hash: H256,
    /// Limit hash of the storage range (inclusive)
    pub limit_hash: H256,
    /// Maximum response size in bytes
    pub response_bytes: u64,
}

/// Request bytecodes by their hashes.
#[derive(Debug, Clone)]
pub struct GetByteCodes {
    /// Request ID - the responding peer must mirror this value
    pub id: u64,
    /// List of code hashes to retrieve
    pub hashes: Vec<H256>,
    /// Maximum response size in bytes
    pub bytes: u64,
}

/// Request trie nodes from state or storage tries.
#[derive(Debug, Clone)]
pub struct GetTrieNodes {
    /// Request ID - the responding peer must mirror this value
    pub id: u64,
    /// State root hash to query against
    pub root_hash: H256,
    /// Paths to trie nodes: [[acc_path, slot_path_1, slot_path_2,...]...]
    /// Paths can be full paths (hash) or partial paths (compact-encoded nibbles)
    pub paths: Vec<Vec<Bytes>>,
    /// Maximum response size in bytes
    pub bytes: u64,
}

// =============================================================================
// RESPONSE MESSAGES
// =============================================================================

/// Response containing a range of accounts.
#[derive(Debug, Clone)]
pub struct AccountRange {
    /// Request ID - mirrors the value from the request
    pub id: u64,
    /// List of accounts in the range
    pub accounts: Vec<AccountRangeUnit>,
    /// Merkle proof for the returned range
    pub proof: Vec<Bytes>,
}

/// Response containing storage ranges for accounts.
#[derive(Debug, Clone)]
pub struct StorageRanges {
    /// Request ID - mirrors the value from the request
    pub id: u64,
    /// Storage slots for each requested account
    pub slots: Vec<Vec<StorageSlot>>,
    /// Merkle proof for the returned range
    pub proof: Vec<Bytes>,
}

/// Response containing bytecodes.
#[derive(Debug, Clone)]
pub struct ByteCodes {
    /// Request ID - mirrors the value from the request
    pub id: u64,
    /// Contract bytecodes
    pub codes: Vec<Bytes>,
}

/// Response containing trie nodes.
#[derive(Debug, Clone)]
pub struct TrieNodes {
    /// Request ID - mirrors the value from the request
    pub id: u64,
    /// Trie nodes
    pub nodes: Vec<Bytes>,
}

// =============================================================================
// snap/2 REQUEST / RESPONSE MESSAGES (EIP-8189)
// =============================================================================

/// snap/2 request: fetch block access lists by block hash.
///
/// Code = 0x08 (offset-relative). Replaces `GetTrieNodes` (0x06) in snap/2.
#[derive(Debug, Clone)]
pub struct GetBlockAccessLists {
    /// Request ID — the responding peer must mirror this value.
    pub id: u64,
    /// Ordered list of block hashes whose BALs are requested.
    pub block_hashes: Vec<H256>,
    /// Soft cap on the response size in bytes.
    pub response_bytes: u64,
}

/// snap/2 response: block access lists corresponding to a `GetBlockAccessLists` request.
///
/// Code = 0x09 (offset-relative). Replaces `TrieNodes` (0x07) in snap/2.
///
/// Wire shape: `Vec<Option<BlockAccessList>>` — position-correspondent with
/// the request's `block_hashes`. A `None` entry means the BAL is unavailable
/// (block unknown or pruned). See module-level doc for the encoding decision.
#[derive(Debug, Clone)]
pub struct BlockAccessLists {
    /// Request ID — mirrors the value from the request.
    pub id: u64,
    /// BALs in the same order as the request's `block_hashes`.
    /// `None` = unavailable (block unknown or BAL pruned).
    pub bals: Vec<Option<BlockAccessList>>,
}

// =============================================================================
// HELPER TYPES
// =============================================================================

/// A single account entry in an AccountRange response.
#[derive(Debug, Clone)]
pub struct AccountRangeUnit {
    /// Hash of the account address
    pub hash: H256,
    /// Account state
    pub account: AccountState,
}

/// A single storage slot entry.
#[derive(Debug, Clone)]
pub struct StorageSlot {
    /// Hash of the storage key
    pub hash: H256,
    /// Storage value
    pub data: U256,
}
