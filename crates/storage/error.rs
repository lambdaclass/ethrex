use ethrex_binary_trie::BinaryTrieError;
use ethrex_common::H256;
use ethrex_common::types::BlockNumber;
use ethrex_common::types::pbt_state::PbtStateError;
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
    /// The binary trie itself failed, on a path that does not go
    /// through the state model — a commit, or a read of a stored node.
    #[error(transparent)]
    BinaryTrie(#[from] BinaryTrieError),
    /// Mapping ethrex account state onto binary trie leaves failed.
    #[error(transparent)]
    PbtState(#[from] PbtStateError),
    /// Shadow tracking reached a block whose parent has no recorded
    /// binary-trie root.
    ///
    /// On a chain with `binaryTreeTime` scheduled this is fatal, not
    /// recoverable: every block from genesis onwards must have advanced the
    /// binary trie, so a gap means the shadow state is incomplete and the
    /// commitment could not be honoured at activation. Seeding from an empty
    /// trie instead would hide the gap until the flip block, where it would
    /// halt the chain with no remedy.
    #[error(
        "no binary-trie root recorded for parent block {parent_hash:#x}: the binary trie is incomplete and cannot be extended (a chain with binaryTreeTime scheduled must have processed every block from genesis)"
    )]
    MissingBinaryTrieRoot { parent_hash: H256 },
    #[error("missing store: is an execution DB being used instead?")]
    MissingStore,
    /// A read was requested against a state root this node does not hold.
    ///
    /// ethrex keeps one version of the state trie on disk plus a bounded chain
    /// of in-memory diff layers, so state older than the retention window (and
    /// state on abandoned forks) is simply gone. Because trie nodes are keyed by
    /// path rather than by hash, reading at such a root would otherwise silently
    /// answer from whatever version the on-disk trie currently holds.
    ///
    /// The message deliberately mirrors the one `StoreVmDatabase::new` produces
    /// for the same condition, so `eth_call` and the account-reading RPCs report
    /// an unavailable state identically.
    #[error(
        "state root missing for block {} (state_root {state_root:#x})",
        block.map_or_else(|| "<unknown>".to_string(), |number| number.to_string())
    )]
    MissingStateRoot {
        block: Option<BlockNumber>,
        state_root: H256,
    },
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
    #[error(
        "Cannot migrate the database: its version is unavailable, which means it predates versioning and migrations. A full resync (removedb) is required."
    )]
    NotFoundDBVersion,
    #[error("Incompatible DB Version: found v{found}, expected v{expected}")]
    IncompatibleDBVersion { found: u64, expected: u64 },
    #[error("Migration from v{from} to v{to} failed: {reason}")]
    MigrationFailed { from: u64, to: u64, reason: String },
}
