use std::sync::Arc;

use ethrex_common::H256;
use ethrex_common::types::AccountUpdate;
use ethrex_state_backend::{MerkleOutput, StateError};
use ethrex_trie::{MptMerkleizer, StorageTrieOpener, Trie, TrieError};

/// Backend-agnostic merkleizer enum used by `ethrex-storage`.
///
/// New variants (e.g. `Binary`) will be added in future PRs.
pub enum Merkleizer {
    Mpt(MptMerkleizer),
    // PR 2: Binary(BinaryMerkleizer),
}

impl Merkleizer {
    /// Create a standard (streaming) MPT merkleizer.
    pub fn new_mpt(
        parent_state_root: H256,
        precompute_witnesses: bool,
        state_trie_opener: Arc<dyn Fn() -> Result<Trie, TrieError> + Send + Sync>,
        storage_trie_opener: Arc<dyn StorageTrieOpener>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        MptMerkleizer::new(
            parent_state_root,
            precompute_witnesses,
            state_trie_opener,
            storage_trie_opener,
            pool,
        )
        .map(Merkleizer::Mpt)
    }

    /// Create a BAL-optimized MPT merkleizer.
    pub fn new_bal_mpt(
        parent_state_root: H256,
        precompute_witnesses: bool,
        state_trie_opener: Arc<dyn Fn() -> Result<Trie, TrieError> + Send + Sync>,
        storage_trie_opener: Arc<dyn StorageTrieOpener>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        MptMerkleizer::new_bal(
            parent_state_root,
            precompute_witnesses,
            state_trie_opener,
            storage_trie_opener,
            pool,
        )
        .map(Merkleizer::Mpt)
    }

    /// Feed a batch of account updates to the merkleizer.
    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError> {
        match self {
            Merkleizer::Mpt(m) => m.feed_updates(updates),
        }
    }

    /// Finalize merkleization and return the root hash together with node diffs.
    pub fn finalize(self) -> Result<MerkleOutput, StateError> {
        match self {
            Merkleizer::Mpt(m) => m.finalize(),
        }
    }
}
