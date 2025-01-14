use ethrex_core::types::BlockHash;
use ethrex_storage::Store;

pub struct StoreWrapper {
    pub store: Store,
    pub block_hash: BlockHash,
}
