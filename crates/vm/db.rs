use ethrex_common::types::BlockHash;
use ethrex_storage::Store;

use crate::backends::exec_db::ExecutionDB;

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum StoreWrapper {
    Store(Store, BlockHash),
    Execution(ExecutionDB, BlockHash),
}
