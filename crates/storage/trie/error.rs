use ethereum_rust_rlp::error::RLPDecodeError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrieError {
    #[error("Libmdbx error: {0}")]
    LibmdbxError(anyhow::Error),
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Verification Error: {0}")]
    Verify(String),
}
