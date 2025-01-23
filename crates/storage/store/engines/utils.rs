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
    // TODO (#307): Remove TotalDifficulty.
    LatestTotalDifficulty = 6,
}

impl From<u8> for ChainDataIndex {
    fn from(value: u8) -> Self {
        match value {
            x if x == ChainDataIndex::ChainConfig as u8 => ChainDataIndex::ChainConfig,
            x if x == ChainDataIndex::EarliestBlockNumber as u8 => {
                ChainDataIndex::EarliestBlockNumber
            }
            x if x == ChainDataIndex::FinalizedBlockNumber as u8 => {
                ChainDataIndex::FinalizedBlockNumber
            }
            x if x == ChainDataIndex::SafeBlockNumber as u8 => ChainDataIndex::SafeBlockNumber,
            x if x == ChainDataIndex::LatestBlockNumber as u8 => ChainDataIndex::LatestBlockNumber,
            x if x == ChainDataIndex::PendingBlockNumber as u8 => {
                ChainDataIndex::PendingBlockNumber
            }
            x if x == ChainDataIndex::LatestTotalDifficulty as u8 => {
                ChainDataIndex::LatestTotalDifficulty
            }
            _ => panic!("Invalid value when casting to ChainDataIndex: {}", value),
        }
    }
}

/// Represents the key for each unique value of the snap state stored in the db
//  Stores the snap state from previous sync cycles. Currently stores the header & state trie download checkpoint
//, but will later on also include the body download checkpoint and the last pivot used
#[derive(Debug, Copy, Clone)]
pub enum SnapStateIndex {
    // Hash of the last downloaded header in a previous sync cycle that was aborted
    HeaderDownloadCheckpoint = 0,
    // Current root hash of the latest State Trie (Used for both fetch & heal)
    StateTrieRootCheckpoint = 1,
    // Accounts which storage needs healing
    PendingStorageHealAccounts = 2,
    // Last key fetched from the state trie
    StateTrieKeyCheckpoint = 3,
    // Paths from the state trie in need of healing
    StateHealPaths = 4,
}

impl From<u8> for SnapStateIndex {
    fn from(value: u8) -> Self {
        match value {
            x if x == SnapStateIndex::HeaderDownloadCheckpoint as u8 => {
                SnapStateIndex::HeaderDownloadCheckpoint
            }
            x if x == SnapStateIndex::StateTrieRootCheckpoint as u8 => {
                SnapStateIndex::StateTrieRootCheckpoint
            }
            x if x == SnapStateIndex::PendingStorageHealAccounts as u8 => {
                SnapStateIndex::PendingStorageHealAccounts
            }
            x if x == SnapStateIndex::StateTrieKeyCheckpoint as u8 => {
                SnapStateIndex::StateTrieKeyCheckpoint
            }
            x if x == SnapStateIndex::StateHealPaths as u8 => SnapStateIndex::StateHealPaths,
            _ => panic!("Invalid value when casting to SnapDataIndex: {}", value),
        }
    }
}
