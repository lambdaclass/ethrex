/// Represents the key for each unique value of the chain data stored in the db
//  Stores chain-specific data such as chain id and latest finalized/pending/safe block number
#[derive(Debug, Copy, Clone)]
pub enum ChainDataIndex {
    ChainConfig,
    EarliestBlockNumber,
    FinalizedBlockNumber,
    SafeBlockNumber,
    LatestBlockNumber,
    PendingBlockNumber,
    IsSynced,
}

impl From<&str> for ChainDataIndex {
    fn from(s: &str) -> Self {
        match s {
            "chain_config" => ChainDataIndex::ChainConfig,
            "earliest_block_number" => ChainDataIndex::EarliestBlockNumber,
            "finalized_block_number" => ChainDataIndex::FinalizedBlockNumber,
            "safe_block_number" => ChainDataIndex::SafeBlockNumber,
            "latest_block_number" => ChainDataIndex::LatestBlockNumber,
            "pending_block_number" => ChainDataIndex::PendingBlockNumber,
            "is_synced" => ChainDataIndex::IsSynced,
            _ => panic!("Invalid value when casting to ChainDataIndex: {}", s),
        }
    }
}

/// Represents the key for each unique value of the snap state stored in the db
//  Stores the snap state from previous sync cycles. Currently stores the header & state trie download checkpoint
//, but will later on also include the body download checkpoint and the last pivot used
#[derive(Debug, Copy, Clone)]
pub enum SnapStateIndex {
    // Hash of the last downloaded header in a previous sync cycle that was aborted
    HeaderDownloadCheckpoint,
    // Paths from the storage trie in need of healing, grouped by hashed account address
    StorageHealPaths,
    // Last key fetched from the state trie
    StateTrieKeyCheckpoint,
    // Paths from the state trie in need of healing
    StateHealPaths,
    // Trie Rebuild Checkpoint (Current State Trie Root, Last Inserted Key Per Segment)
    StateTrieRebuildCheckpoint,
    // Storage tries awaiting rebuild (AccountHash, ExpectedRoot)
    StorageTrieRebuildPending,
}

impl From<&str> for SnapStateIndex {
    fn from(s: &str) -> Self {
        match s {
            "header_download_checkpoint" => Self::HeaderDownloadCheckpoint,
            "storage_heal_paths" => Self::StorageHealPaths,
            "state_trie_key_checkpoint" => Self::StateTrieKeyCheckpoint,
            "state_heal_paths" => Self::StateHealPaths,
            "state_trie_rebuild_checkpoint" => Self::StateTrieRebuildCheckpoint,
            "storage_trie_rebuild_pending" => Self::StorageTrieRebuildPending,
            _ => panic!("Invalid SnapStateIndex string: {}", s),
        }
    }
}

