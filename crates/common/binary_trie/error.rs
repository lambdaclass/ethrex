use thiserror::Error;

#[derive(Debug, Error)]
pub enum BinaryTrieError {
    #[error("trie traversal exceeded maximum depth of 248")]
    MaxDepthExceeded,
    #[error("key must be exactly 32 bytes")]
    InvalidKeyLength,
    #[error("stem must be exactly 31 bytes")]
    InvalidStemLength,
    #[error("node {0} not found in store")]
    NodeNotFound(u64),
    #[error("node store I/O error: {0}")]
    StoreError(String),
    #[error("invalid node encoding: {0}")]
    DeserializationError(String),
}
