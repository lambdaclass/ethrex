use ethrex_storage::Store;
use serde_json::Value;

use crate::utils::{RpcErr, RpcRequest};

const ETHREX_PKG_NAME: &str = env!("CARGO_PKG_NAME");
const ETHREX_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const ETHREX_COMMIT_HASH: &str = env!("VERGEN_RUSTC_COMMIT_HASH");
const ETHREX_BUILD_OS: &str = env!("VERGEN_RUSTC_HOST_TRIPLE");
const ETHREX_RUSTC_VERSION: &str = env!("VERGEN_RUSTC_SEMVER");

pub fn client_version(_req: &RpcRequest, _store: Store) -> Result<Value, RpcErr> {
    Ok(Value::String(format!(
        "{}/v{}-{}/{}/rustc-v{}",
        ETHREX_PKG_NAME,
        ETHREX_PKG_VERSION,
        &ETHREX_COMMIT_HASH[0..6],
        ETHREX_BUILD_OS,
        ETHREX_RUSTC_VERSION
    )))
}
