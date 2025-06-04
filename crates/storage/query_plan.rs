use crate::{error::StoreError, Store};
use ethrex_common::{
    types::{Block, Receipt},
    H256,
};
use ethrex_trie::NodeHash;

pub struct QueryPlan {
    pub account_updates: (
        Vec<(NodeHash, Vec<u8>)>,                 // vec<(node_hash, node_data)>
        Vec<(Vec<u8>, Vec<(NodeHash, Vec<u8>)>)>, // hashed_address, vec<(node_hash, node_data)>
    ),
    pub block: Block,
    pub receipts: (H256, Vec<Receipt>),
}

impl QueryPlan {
    pub async fn apply_to_store(self, store: Store) -> Result<(), StoreError> {
        store.store_changes(self).await?;
        Ok(())
    }
}
