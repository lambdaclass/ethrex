use crate::H256;

use super::BlobsBundle;

#[derive(Clone)]
pub struct Batch {
    pub number: u64,
    pub first_block: u64,
    pub last_block: u64,
    pub state_root: H256,
    pub deposit_logs_hash: H256,
    pub message_hashes: Vec<H256>,
    pub blobs_bundle: BlobsBundle,
}
