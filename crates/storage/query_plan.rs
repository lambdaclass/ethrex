use crate::error::StoreError;
use ethrex_common::{
    types::{Block, Receipt},
    H256,
};
use ethrex_trie::NodeHash;

pub struct QueryPlan {
    pub account_updates: (
        Vec<(NodeHash, Vec<u8>)>,
        Vec<(Vec<u8>, Vec<(NodeHash, Vec<u8>)>)>,
    ),
    pub block: Block,
    pub receipts: (H256, Vec<Receipt>),
}

impl QueryPlan {
    pub fn apply_to_store(self) -> Result<(), StoreError> {
        Ok(())
    }
}
