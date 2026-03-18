use thiserror::Error;

#[derive(Debug, Error)]
pub enum BinaryTrieError {
    #[error("trie traversal exceeded maximum depth of 248")]
    MaxDepthExceeded,
    #[error("key must be exactly 32 bytes")]
    InvalidKeyLength,
    #[error("stem must be exactly 31 bytes")]
    InvalidStemLength,
}
