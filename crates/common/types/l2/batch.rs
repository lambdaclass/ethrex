use crate::{
    H256,
    types::{BlobsBundle, l2_to_l2_message::L2toL2Message},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Batch {
    pub number: u64,
    pub first_block: u64,
    pub last_block: u64,
    pub state_root: H256,
    pub privileged_transactions_hash: H256,
    pub l1_message_hashes: Vec<H256>,
    pub l2_to_l2_messages: Vec<L2toL2Message>,
    pub blobs_bundle: BlobsBundle,
    pub commit_tx: Option<H256>,
    pub verify_tx: Option<H256>,
}
