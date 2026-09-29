/// Represents the key for each unique value of the chain data stored in the db
//  Stores chain-specific data such as chain id and latest finalized/pending/safe block number
#[derive(Debug, Copy, Clone)]
pub enum ChainDataIndex {
    ChainConfig = 0,
    EarliestBlockNumber = 1,
    FinalizedBlockNumber = 2,
    SafeBlockNumber = 3,
    LatestBlockNumber = 4,
    PendingBlockNumber = 5,
}

/// Represents the key for each unique value of the snap state stored in the db
//  Stores the snap state from previous sync cycles. Currently stores the header & state trie download checkpoint
//, but will later on also include the body download checkpoint and the last pivot used
#[derive(Debug, Copy, Clone)]
pub enum SnapStateIndex {
    // Hash of the last downloaded header in a previous sync cycle that was aborted
    HeaderDownloadCheckpoint = 0,
    // Last key fetched from the state trie
    StateTrieKeyCheckpoint = 1,
    // Paths from the state trie in need of healing
    StateHealPaths = 2,
    // Trie Rebuild Checkpoint (Current State Trie Root, Last Inserted Key Per Segment)
    StateTrieRebuildCheckpoint = 3,
    // Storage tries awaiting rebuild (AccountHash, ExpectedRoot)
    StorageTrieRebuildPending = 4,
}
