use ethrex_common::H256;
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
    #[error("Failed to generate snapshot: {0}")]
    SnapshotGeneration(#[from] SnapshotGenerationError),
}

#[derive(thiserror::Error, Debug)]
pub enum SnapshotGenerationError {
    #[error("Failed to open account state trie with error: {0}")]
    FailedToOpenAccountStateTrie(#[source] Box<StoreError>),
    #[error("Failed to decode account state trie from path {0:#x} with error: {1}")]
    FailedToDecodeAccountState(H256, #[source] RLPDecodeError),
    #[error(
        "Failed to open account state storage trie for account hash {0:#x} from path {1:#x} with error: {2}"
    )]
    FailedToOpenAccountStateStorageTrie(H256, H256, #[source] Box<StoreError>),
    #[error(
        "Failed to store account state storage nodes batch for account hash {0:#x} with error: {1}"
    )]
    FailedToStoreAccountStateStorageNodesBatch(H256, #[source] TrieError),
    #[error(
        "Failed to store remaining account state storage nodes for account hash {0:#x} with error: {1}"
    )]
    FailedToStoreRemainingAccountStateStorageNodes(H256, #[source] TrieError),
    #[error("Failed to store account state nodes batch for account hash {0:#x} with error: {1}")]
    FailedToStoreAccountStateNodesBatch(H256, #[source] TrieError),
    #[error(
        "Failed to store remaining account state nodes with error: {0}
    "
    )]
    FailedToStoreRemainingAccountStateNodes(#[source] TrieError),
}
