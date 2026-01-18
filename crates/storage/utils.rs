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
    // Current snap sync phase
    CurrentPhase = 5,
    // Full checkpoint data (serialized SnapSyncCheckpoint)
    FullCheckpoint = 6,
    // Pivot block info (number, hash, state_root)
    PivotBlockInfo = 7,
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
            6 => SnapStateIndex::FullCheckpoint,
            7 => SnapStateIndex::PivotBlockInfo,
            _ => panic!("Invalid value when casting to SnapDataIndex: {value}"),
        }
    }
}

/// Represents the current phase of snap sync.
/// Used for checkpointing to enable resume from any phase.
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

/// Checkpoint for snap sync progress that can be persisted to disk.
/// Enables resuming snap sync from any phase after a restart.
#[derive(Debug, Clone, Default)]
pub struct SnapSyncCheckpoint {
    /// Current phase of snap sync
    pub phase: SnapSyncPhase,
    /// Pivot block number
    pub pivot_block_number: u64,
    /// Pivot block hash
    pub pivot_block_hash: ethrex_common::H256,
    /// Pivot state root
    pub pivot_state_root: ethrex_common::H256,
    /// Number of account snapshot files processed
    pub account_files_processed: usize,
    /// Number of storage snapshot files processed
    pub storage_files_processed: usize,
    /// Number of nodes healed during state healing
    pub state_nodes_healed: u64,
    /// Number of nodes healed during storage healing
    pub storage_nodes_healed: u64,
    /// Number of bytecodes downloaded
    pub bytecodes_downloaded: usize,
    /// Timestamp when checkpoint was created
    pub checkpoint_timestamp: u64,
}

impl SnapSyncCheckpoint {
    /// Creates a new checkpoint at the given phase
    pub fn new(phase: SnapSyncPhase) -> Self {
        Self {
            phase,
            checkpoint_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            ..Default::default()
        }
    }

    /// Sets the pivot block information
    pub fn with_pivot(
        mut self,
        block_number: u64,
        block_hash: ethrex_common::H256,
        state_root: ethrex_common::H256,
    ) -> Self {
        self.pivot_block_number = block_number;
        self.pivot_block_hash = block_hash;
        self.pivot_state_root = state_root;
        self
    }

    /// Updates the checkpoint timestamp to now
    pub fn touch(&mut self) {
        self.checkpoint_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
    }

    /// Returns true if the checkpoint is stale (older than max_age_secs)
    pub fn is_stale(&self, max_age_secs: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_sub(self.checkpoint_timestamp) > max_age_secs
    }

    /// RLP encode the checkpoint for storage
    pub fn encode_to_vec(&self) -> Vec<u8> {
        use ethrex_rlp::encode::RLPEncode;
        let mut encoded = Vec::new();
        let group1 = (
            self.phase as u8,
            self.pivot_block_number,
            self.pivot_block_hash,
            self.pivot_state_root,
        );
        let group2 = (
            self.account_files_processed as u64,
            self.storage_files_processed as u64,
            self.state_nodes_healed,
            self.storage_nodes_healed,
        );
        let group3 = (
            self.bytecodes_downloaded as u64,
            self.checkpoint_timestamp,
        );
        (group1, group2, group3).encode(&mut encoded);
        encoded
    }

    /// RLP decode the checkpoint from storage
    pub fn decode(bytes: &[u8]) -> Result<Self, ethrex_rlp::error::RLPDecodeError> {
        use ethrex_rlp::decode::RLPDecode;
        let (group1, group2, group3): (
            (u8, u64, ethrex_common::H256, ethrex_common::H256),
            (u64, u64, u64, u64),
            (u64, u64),
        ) = RLPDecode::decode(bytes)?;

        let (phase_u8, pivot_block_number, pivot_block_hash, pivot_state_root) = group1;
        let (account_files_processed, storage_files_processed, state_nodes_healed, storage_nodes_healed) = group2;
        let (bytecodes_downloaded, checkpoint_timestamp) = group3;

        Ok(Self {
            phase: SnapSyncPhase::from(phase_u8),
            pivot_block_number,
            pivot_block_hash,
            pivot_state_root,
            account_files_processed: account_files_processed as usize,
            storage_files_processed: storage_files_processed as usize,
            state_nodes_healed,
            storage_nodes_healed,
            bytecodes_downloaded: bytecodes_downloaded as usize,
            checkpoint_timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snap_sync_checkpoint_roundtrip() {
        let checkpoint = SnapSyncCheckpoint {
            phase: SnapSyncPhase::AccountInsertion,
            pivot_block_number: 12345678,
            pivot_block_hash: ethrex_common::H256::repeat_byte(0xab),
            pivot_state_root: ethrex_common::H256::repeat_byte(0xcd),
            account_files_processed: 42,
            storage_files_processed: 17,
            state_nodes_healed: 1000,
            storage_nodes_healed: 2000,
            bytecodes_downloaded: 500,
            checkpoint_timestamp: 1700000000,
        };

        let encoded = checkpoint.encode_to_vec();
        let decoded = SnapSyncCheckpoint::decode(&encoded).expect("decode should succeed");

        assert_eq!(decoded.phase, SnapSyncPhase::AccountInsertion);
        assert_eq!(decoded.pivot_block_number, 12345678);
        assert_eq!(decoded.pivot_block_hash, ethrex_common::H256::repeat_byte(0xab));
        assert_eq!(decoded.pivot_state_root, ethrex_common::H256::repeat_byte(0xcd));
        assert_eq!(decoded.account_files_processed, 42);
        assert_eq!(decoded.storage_files_processed, 17);
        assert_eq!(decoded.state_nodes_healed, 1000);
        assert_eq!(decoded.storage_nodes_healed, 2000);
        assert_eq!(decoded.bytecodes_downloaded, 500);
        assert_eq!(decoded.checkpoint_timestamp, 1700000000);
    }

    #[test]
    fn test_snap_sync_phase_roundtrip() {
        for phase_val in 0..=9u8 {
            let phase = SnapSyncPhase::from(phase_val);
            assert_eq!(phase as u8, phase_val);
        }
    }

    #[test]
    fn test_checkpoint_staleness() {
        let mut checkpoint = SnapSyncCheckpoint::new(SnapSyncPhase::AccountDownload);
        assert!(!checkpoint.is_stale(60));

        checkpoint.checkpoint_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 7200;

        assert!(checkpoint.is_stale(3600));
        assert!(!checkpoint.is_stale(10800));
    }
}
