use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use secp256k1::{Message, SecretKey};
pub struct L2ConnState {
    latest_block_sent: u64,
    latest_batch_sent: u64,
    store_rollup: StoreRollup,
    commiter_key: Option<SecretKey>,
}
