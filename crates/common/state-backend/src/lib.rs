//! Backend-agnostic state abstraction for ethrex.
//!
//! Defines the traits ([`StateReader`], [`StateCommitter`]) and shared types
//! ([`MerkleOutput`], [`NodeUpdates`], [`StateError`], [`BackendKind`],
//! [`CodeReader`]) that concrete trie backends implement. No concrete
//! backends live here -- see `ethrex-trie` for `MptBackend` and
//! `ethrex-storage` for the `StateBackend` enum that assembles them.

use std::sync::Arc;

use ethereum_types::{Address, H256};
use ethrex_common::types::{AccountInfo, AccountUpdate, Code};

// --- Backend kind ---

/// Identifies which trie backend is active.
///
/// Single variant today (`Mpt`). Adding a new variant forces a compiler-guided
/// audit of every exhaustive match in the storage layer, ensuring no backend
/// path is silently skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Mpt,
}

/// Closure type for reading code by hash from the database.
///
/// Lives here (not in `ethrex-trie`) so it can be used across crates without
/// the `ethrex-trie` dependency. The error type is `StateError` to match the
/// backend trait surface.
pub type CodeReader = Arc<dyn Fn(H256) -> Result<Option<Vec<u8>>, StateError> + Send + Sync>;

// --- Mutation types ---

#[derive(Clone, Debug)]
pub struct CodeMut {
    pub code: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct AccountMut {
    pub account: Option<AccountInfo>,
    pub code: Option<CodeMut>,
    /// Current total code size. MPT ignores it; other backends may encode it
    /// into their leaf layout. Populated by callers so the interface is stable
    /// across backends.
    pub code_size: usize,
}

// --- Output types ---

/// Output of merkleization. Produced by the `Merkleizer` enum in `ethrex-storage`.
pub struct MerkleOutput {
    pub root: H256,
    /// Backend-specific node diffs for the storage layer.
    pub node_updates: NodeUpdates,
    /// Code deployments.
    pub code_updates: Vec<(H256, Code)>,
    /// Accumulated account updates for witness pre-computation.
    /// Populated when precompute_witnesses is enabled.
    pub accumulated_updates: Option<Vec<AccountUpdate>>,
}

#[expect(clippy::type_complexity)]
pub enum NodeUpdates {
    Mpt {
        /// State trie node changes: (nibble_path_bytes, rlp_node).
        state_updates: Vec<(Vec<u8>, Vec<u8>)>,
        /// Per-account storage trie changes.
        storage_updates: Vec<(H256, Vec<(Vec<u8>, Vec<u8>)>)>,
    },
}

// --- Error ---

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("trie error: {0}")]
    Trie(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("other: {0}")]
    Other(String),
}

// --- Traits ---

/// Point reads. Used by the EVM, RPC handlers, and any read-only consumer.
pub trait StateReader {
    fn account(&self, addr: Address) -> Result<Option<AccountInfo>, StateError>;
    fn storage(&self, addr: Address, slot: H256) -> Result<H256, StateError>;
    fn code(&self, addr: Address, code_hash: H256) -> Result<Option<Vec<u8>>, StateError>;
}

/// Used for non-pipelined code paths (genesis, snap sync, tests).
/// For the pipelined block execution path, use the `Merkleizer` enum instead.
pub trait StateCommitter: StateReader {
    fn update_accounts(&mut self, addrs: &[Address], muts: &[AccountMut])
    -> Result<(), StateError>;
    fn update_storage(&mut self, addr: Address, slots: &[(H256, H256)]) -> Result<(), StateError>;
    /// Wipe all storage for an account (SELFDESTRUCT semantics).
    fn clear_storage(&mut self, addr: Address) -> Result<(), StateError>;
    fn hash(&mut self) -> Result<H256, StateError>;
    fn commit(self) -> Result<MerkleOutput, StateError>;
}
