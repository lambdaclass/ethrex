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

/// Represents the key for each unique value of the chain data stored in the db
//  Stores chain-specific data such as chain id and latest finalized/pending/safe block number
#[derive(Debug, Copy, Clone)]
pub enum SnapStateIndex {
    // Pivot used by the last completed snap sync cycle
    LastPivot = 0,
    // Hash of the last downloaded header in a previous sync that was aborted
    LatestDownloadedHeader = 1,
    // Hash of the last downloaded body in a previous sync that was aborted
    LatestDownloadedBody = 2,
}

impl From<u8> for SnapStateIndex {
    fn from(value: u8) -> Self {
        match value {
            x if x == SnapStateIndex::LastPivot as u8 => SnapStateIndex::LastPivot,
            x if x == SnapStateIndex::LatestDownloadedHeader as u8 => {
                SnapStateIndex::LatestDownloadedHeader
            }
            x if x == SnapStateIndex::LatestDownloadedBody as u8 => {
                SnapStateIndex::LatestDownloadedBody
            }
            _ => panic!("Invalid value when casting to SnapDataIndex: {}", value),
        }
    }
}
