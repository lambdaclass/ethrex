use super::{BlobsBundle, Block, block_access_list::BlockAccessList, requests::EncodedRequests};
use ethereum_types::U256;

#[derive(Debug, Clone)]
pub struct PayloadBundle {
    pub block: Block,
    pub block_value: U256,
    pub blobs_bundle: BlobsBundle,
    pub requests: Vec<EncodedRequests>,
    /// Block Access List for EIP-7928
    pub block_access_list: Option<BlockAccessList>,
}

impl PayloadBundle {
    pub fn from_block(block: Block) -> Self {
        PayloadBundle {
            block,
            block_value: U256::zero(),
            blobs_bundle: BlobsBundle::empty(),
            requests: Vec::default(),
            block_access_list: None,
        }
    }
}
