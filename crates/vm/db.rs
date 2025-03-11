use ethrex_common::types::BlockHash;
use ethrex_storage::Store;

#[derive(Clone)]
pub struct StoreWrapper {
    pub store: Store,
    pub block_hash: BlockHash,
}
