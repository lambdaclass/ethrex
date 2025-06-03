use ethrex_common::H160;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::TrieError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    TrieError(#[from] TrieError),
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Missing storage trie for address {0}")]
    MissingStorageTrie(H160),
    #[error("Missing storage for address {0}")]
    StorageNotFound(H160),
}
