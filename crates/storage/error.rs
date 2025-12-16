use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::TrieError;
use thiserror::Error;

// TODO improve errors
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("DecodeError")]
    DecodeError,
    #[cfg(feature = "rocksdb")]
    #[error("Rocksdb error: {0}")]
    RocksdbError(#[from] rocksdb::Error),
    #[error("{0}")]
    Custom(String),
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error(transparent)]
    Trie(#[from] TrieError),
    #[error("missing store: is an execution DB being used instead?")]
    MissingStore,
    #[error("Could not open DB for reading")]
    ReadError,
    #[error("Could not instantiate cursor for table {0}")]
    CursorError(String),
    #[error("Missing latest block number")]
    MissingLatestBlockNumber,
    #[error("Missing earliest block number")]
    MissingEarliestBlockNumber,
    #[error("Failed to lock mempool for writing")]
    MempoolWriteLock(String),
    #[error("Failed to lock mempool for reading")]
    MempoolReadLock(String),
    #[error("Failed to lock database for writing")]
    LockError,
    #[error("Incompatible chain configuration")]
    IncompatibleChainConfig,
    #[error("Failed to convert index: {0}")]
    TryInto(#[from] std::num::TryFromIntError),
    #[error("Update batch contains no blocks")]
    UpdateBatchNoBlocks,
    #[error("Pivot changed")]
    PivotChanged,
    #[error("Error reading from disk: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Error serializing metadata: {0}")]
    DbMetadataError(#[from] serde_json::Error),
    #[error("Incompatible DB Version: not found, expected v{expected}")]
    NotFoundDBVersion { expected: u64 },
    #[error("Incompatible DB Version: found v{found}, expected v{expected}")]
    IncompatibleDBVersion { found: u64, expected: u64 },
}
