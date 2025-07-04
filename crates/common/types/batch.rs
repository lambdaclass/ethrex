use serde::{Deserialize, Serialize};

use crate::H256;

use super::BlobsBundle;

#[derive(Clone, Serialize, Deserialize)]
pub struct Batch {
    pub number: u64,
    pub first_block: u64,
    pub last_block: u64,
    pub state_root: H256,
    pub privileged_transactions_hash: H256,
    pub message_hashes: Vec<H256>,
    #[serde(skip_serializing, skip_deserializing)]
    pub blobs_bundle: BlobsBundle,
    pub commit_tx: Option<H256>,
    pub verify_tx: Option<H256>,
}
