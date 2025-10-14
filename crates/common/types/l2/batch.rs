use crate::{H256, types::BlobsBundle};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Batch {
    pub number: u64,
    pub first_block: u64,
    pub last_block: u64,
    pub state_root: H256,
    pub privileged_transactions_hash: H256,
    pub message_hashes: Vec<H256>,
    pub blobs_bundle: BlobsBundle,
    pub commit_tx: Option<H256>,
    pub verify_tx: Option<H256>,
}
