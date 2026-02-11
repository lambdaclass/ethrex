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
