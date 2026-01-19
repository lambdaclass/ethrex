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
            _ => panic!("Invalid value when casting to ChainDataIndex: {value}"),
        }
    }
}

/// Represents the key for each unique value of the snap state stored in the db
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
    // Current snap sync phase
    CurrentPhase = 5,
    // Pivot block info (number, hash, state_root)
    PivotBlockInfo = 6,
}

impl From<u8> for SnapStateIndex {
    fn from(value: u8) -> Self {
        match value {
            0 => SnapStateIndex::HeaderDownloadCheckpoint,
            1 => SnapStateIndex::StateTrieKeyCheckpoint,
            2 => SnapStateIndex::StateHealPaths,
            3 => SnapStateIndex::StateTrieRebuildCheckpoint,
            4 => SnapStateIndex::StorageTrieRebuildPending,
            5 => SnapStateIndex::CurrentPhase,
            6 => SnapStateIndex::PivotBlockInfo,
            _ => panic!("Invalid value when casting to SnapStateIndex: {value}"),
        }
    }
}

/// Represents the current phase of snap sync.
/// Used for progress tracking and logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SnapSyncPhase {
    /// Snap sync has not started yet
    #[default]
    NotStarted = 0,
    /// Downloading block headers
    HeaderDownload = 1,
    /// Downloading account ranges from peers
    AccountDownload = 2,
    /// Inserting downloaded accounts into the trie
    AccountInsertion = 3,
    /// Downloading storage ranges from peers
    StorageDownload = 4,
    /// Inserting downloaded storage into the trie
    StorageInsertion = 5,
    /// Healing the state trie
    StateHealing = 6,
    /// Healing storage tries
    StorageHealing = 7,
    /// Downloading bytecode
    BytecodeDownload = 8,
    /// Snap sync completed successfully
    Completed = 9,
}

impl From<u8> for SnapSyncPhase {
    fn from(value: u8) -> Self {
        match value {
            0 => SnapSyncPhase::NotStarted,
            1 => SnapSyncPhase::HeaderDownload,
            2 => SnapSyncPhase::AccountDownload,
            3 => SnapSyncPhase::AccountInsertion,
            4 => SnapSyncPhase::StorageDownload,
            5 => SnapSyncPhase::StorageInsertion,
            6 => SnapSyncPhase::StateHealing,
            7 => SnapSyncPhase::StorageHealing,
            8 => SnapSyncPhase::BytecodeDownload,
            9 => SnapSyncPhase::Completed,
            _ => panic!("Invalid value when casting to SnapSyncPhase: {value}"),
        }
    }
}

impl std::fmt::Display for SnapSyncPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapSyncPhase::NotStarted => write!(f, "NotStarted"),
            SnapSyncPhase::HeaderDownload => write!(f, "HeaderDownload"),
            SnapSyncPhase::AccountDownload => write!(f, "AccountDownload"),
            SnapSyncPhase::AccountInsertion => write!(f, "AccountInsertion"),
            SnapSyncPhase::StorageDownload => write!(f, "StorageDownload"),
            SnapSyncPhase::StorageInsertion => write!(f, "StorageInsertion"),
            SnapSyncPhase::StateHealing => write!(f, "StateHealing"),
            SnapSyncPhase::StorageHealing => write!(f, "StorageHealing"),
            SnapSyncPhase::BytecodeDownload => write!(f, "BytecodeDownload"),
            SnapSyncPhase::Completed => write!(f, "Completed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snap_sync_phase_roundtrip() {
        for phase_val in 0..=9u8 {
            let phase = SnapSyncPhase::from(phase_val);
            assert_eq!(phase as u8, phase_val);
        }
    }
}
