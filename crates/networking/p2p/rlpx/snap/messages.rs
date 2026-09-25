//! Snap protocol message definitions
//!
//! This module contains the message types used in the snap sync protocol.
//! Each message type implements RLPxMessage for encoding/decoding.

use bytes::Bytes;
use ethrex_common::{H256, U256, types::AccountState, types::block_access_list::BlockAccessList};

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

// =============================================================================
// snap/2 REQUEST / RESPONSE MESSAGES (EIP-8189)
// =============================================================================

/// snap/2 request: fetch block access lists by block hash.
/// Code = 0x08 (offset-relative). Replaces `GetTrieNodes` (0x06) in snap/2.
///
/// Wire format (EIP-8189 §"GetBlockAccessLists"):
///   `[request-id, [blockhash1, blockhash2, ...], bytes]`
#[derive(Debug, Clone)]
pub struct Snap2GetBlockAccessLists {
    pub id: u64,
    pub block_hashes: Vec<H256>,
    /// Soft cap on response size in bytes (EIP-8189). Spec recommends 2 MiB default.
    pub response_bytes: u64,
}

/// snap/2 response: block access lists corresponding to a `Snap2GetBlockAccessLists`.
/// Code = 0x09 (offset-relative). Position-correspondent with request.
/// A `None` entry means the BAL is unavailable (block unknown / pruned / pre-Amsterdam).
#[derive(Debug, Clone)]
pub struct Snap2BlockAccessLists {
    pub id: u64,
    pub bals: Vec<Option<BlockAccessList>>,
}
