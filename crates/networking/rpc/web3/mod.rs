use ethrex_storage::Store;
use serde_json::Value;

use crate::utils::{RpcErr, RpcRequest};
use ethrex_common::constants;

pub fn client_version(_req: &RpcRequest, _store: Store) -> Result<Value, RpcErr> {
    Ok(Value::String(format!(
        "{}/v{}-develop-{}/{}/rustc-v{}",
        constants::ETHREX_PKG_NAME,
        constants::ETHREX_PKG_VERSION,
        &constants::ETHREX_COMMIT_HASH[0..6],
        constants::ETHREX_BUILD_OS,
        constants::ETHREX_RUSTC_VERSION
    )))
}
