use std::sync::Arc;

use ethrex_binary_trie::{BinaryMerkleizer, BinaryTrieProvider};
use ethrex_common::H256;
use ethrex_common::types::AccountUpdate;
use ethrex_state_backend::{MerkleOutput, StateError};
use ethrex_trie::{MptMerkleizer, TrieProvider};

/// Backend-agnostic merkleizer enum used by `ethrex-storage`.
///
/// - `Mpt` — standard 16-shard MPT merkleizer.
/// - `Binary` — standard 16-shard binary trie merkleizer.
/// - `Transition` — uses the same `BinaryMerkleizer` as `Binary` because the
///   MPT side is frozen read-only during transition; all merkleization happens
///   in the binary overlay.
pub enum Merkleizer {
    Mpt(MptMerkleizer),
    Binary(BinaryMerkleizer),
    /// Transition mode: MPT side is frozen; all merkleization goes through
    /// the binary overlay. Identical wire to `Binary` — the variant name
    /// documents the active backend kind.
    Transition(BinaryMerkleizer),
}

impl Merkleizer {
    /// Create a standard (streaming) MPT merkleizer.
    pub fn new_mpt(
        parent_state_root: H256,
        precompute_witnesses: bool,
        provider: Arc<dyn TrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        MptMerkleizer::new(parent_state_root, precompute_witnesses, provider, pool)
            .map(Merkleizer::Mpt)
    }

    /// Create a BAL-optimized MPT merkleizer.
    pub fn new_bal_mpt(
        parent_state_root: H256,
        precompute_witnesses: bool,
        provider: Arc<dyn TrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        MptMerkleizer::new_bal(parent_state_root, precompute_witnesses, provider, pool)
            .map(Merkleizer::Mpt)
    }

    /// Create a standard (streaming) binary trie merkleizer.
    pub fn new_binary(
        parent_state_root: H256,
        precompute_witnesses: bool,
        provider: Arc<dyn BinaryTrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        BinaryMerkleizer::new(parent_state_root, precompute_witnesses, provider, pool)
            .map(Merkleizer::Binary)
    }

    /// Create a BAL-optimized binary trie merkleizer.
    pub fn new_bal_binary(
        parent_state_root: H256,
        precompute_witnesses: bool,
        provider: Arc<dyn BinaryTrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        BinaryMerkleizer::new_bal(parent_state_root, precompute_witnesses, provider, pool)
            .map(Merkleizer::Binary)
    }

    /// Create a transition-mode merkleizer.
    ///
    /// The MPT side is frozen during transition; all merkleization happens in
    /// the binary overlay. This wraps `BinaryMerkleizer` under the `Transition`
    /// variant so that callers know the active backend kind.
    ///
    /// `parent_state_root` is the binary overlay root from the previous block
    /// (or `H256::zero()` for the first block after activation).
    pub fn new_transition(
        parent_state_root: H256,
        provider: Arc<dyn BinaryTrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        BinaryMerkleizer::new(parent_state_root, false, provider, pool).map(Merkleizer::Transition)
    }

    /// Feed a batch of account updates to the merkleizer.
    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError> {
        match self {
            Merkleizer::Mpt(m) => m.feed_updates(updates),
            Merkleizer::Binary(m) | Merkleizer::Transition(m) => m.feed_updates(updates),
        }
    }

    /// Finalize merkleization and return the root hash together with node diffs.
    pub fn finalize(self) -> Result<MerkleOutput, StateError> {
        match self {
            Merkleizer::Mpt(m) => m.finalize(),
            Merkleizer::Binary(m) | Merkleizer::Transition(m) => m.finalize(),
        }
    }
}
