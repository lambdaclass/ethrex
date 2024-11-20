use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::TrieError;
use thiserror::Error;

// TODO improve errors
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("DecodeError")]
    DecodeError,
    #[cfg(feature = "libmdbx")]
    #[error("Libmdbx error: {0}")]
    LibmdbxError(anyhow::Error),
    #[error("{0}")]
    Custom(String),
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error(transparent)]
    Trie(#[from] TrieError),
    #[error("missing store: is an execution DB being used instead?")]
    MissingStore,
}
