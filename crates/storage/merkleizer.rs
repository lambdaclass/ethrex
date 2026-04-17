use std::sync::Arc;

use ethrex_common::H256;
use ethrex_common::types::AccountUpdate;
use ethrex_state_backend::{MerkleOutput, StateError};
use ethrex_trie::{MptMerkleizer, TrieProvider};

/// Backend-agnostic merkleizer enum used by `ethrex-storage`.
pub enum Merkleizer {
    Mpt(MptMerkleizer),
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
