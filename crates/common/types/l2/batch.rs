use crate::{
    H256,
    types::{BlobsBundle, balance_diff::BalanceDiff},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Batch {
    pub number: u64,
    pub first_block: u64,
    pub last_block: u64,
    pub state_root: H256,
    pub l1_in_messages_rolling_hash: H256,
    pub l2_in_message_rolling_hashes: Vec<(u64, H256)>,
    pub l1_out_message_hashes: Vec<H256>,
    pub non_privileged_transactions: u64,
    pub balance_diffs: Vec<BalanceDiff>,
    pub blobs_bundle: BlobsBundle,
    pub commit_tx: Option<H256>,
    pub verify_tx: Option<H256>,
}

impl Batch {
    /// Returns true if this batch contains no state-changing transactions.
    ///
    /// An empty batch has: zero non-privileged transactions, no deposit/withdrawal
    /// messages, and no balance diffs. Such batches can be verified on L1 without
    /// a ZK proof because the state root is unchanged.
    pub fn is_empty_batch(&self) -> bool {
        self.non_privileged_transactions == 0
            && self.l1_in_messages_rolling_hash == H256::zero()
            && self.l1_out_message_hashes.is_empty()
            && self.balance_diffs.is_empty()
            && self.l2_in_message_rolling_hashes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_batch_is_empty() {
        let batch = Batch::default();
        assert!(batch.is_empty_batch());
    }

    #[test]
    fn batch_with_non_privileged_transactions_is_not_empty() {
        let batch = Batch {
            non_privileged_transactions: 1,
            ..Default::default()
        };
        assert!(!batch.is_empty_batch());
    }

    #[test]
    fn batch_with_l1_messages_is_not_empty() {
        let batch = Batch {
            l1_in_messages_rolling_hash: H256([0xAA; 32]),
            ..Default::default()
        };
        assert!(!batch.is_empty_batch());
    }

    #[test]
    fn batch_with_withdrawals_is_not_empty() {
        let batch = Batch {
            l1_out_message_hashes: vec![H256([0xBB; 32])],
            ..Default::default()
        };
        assert!(!batch.is_empty_batch());
    }

    #[test]
    fn batch_with_balance_diffs_is_not_empty() {
        let batch = Batch {
            balance_diffs: vec![BalanceDiff::default()],
            ..Default::default()
        };
        assert!(!batch.is_empty_batch());
    }

    #[test]
    fn batch_with_l2_messages_is_not_empty() {
        let batch = Batch {
            l2_in_message_rolling_hashes: vec![(1, H256([0xCC; 32]))],
            ..Default::default()
        };
        assert!(!batch.is_empty_batch());
    }

    #[test]
    fn batch_with_only_blocks_and_state_root_is_empty() {
        // A batch that covers blocks (first_block..last_block) but has
        // zero transactions — this is the typical empty-batch scenario.
        let batch = Batch {
            number: 4,
            first_block: 100,
            last_block: 200,
            state_root: H256([0x11; 32]),
            ..Default::default()
        };
        assert!(batch.is_empty_batch());
    }
}
