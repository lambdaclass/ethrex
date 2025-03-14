use ethrex_common::types::BlockHash;
use ethrex_storage::Store;

use crate::backends::revm::execution_db::ExecutionDB;

#[derive(Clone)]
pub enum StoreWrapper {
    StoreDB(Store, BlockHash),
    ExecutionCache(ExecutionDB, BlockHash),
}
