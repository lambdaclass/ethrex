use ethrex_storage::Store;
use serde_json::Value;

use crate::utils::{RpcErr, RpcRequest};

const ETHREX_PKG_NAME: &str = env!("CARGO_PKG_NAME");
const ETHREX_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn client_version(_req: &RpcRequest, _store: Store) -> Result<Value, RpcErr> {
    Ok(Value::String(format!(
        "{}/v{}-{}/{}/rustc-v{}",
        ETHREX_PKG_NAME,
        ETHREX_PKG_VERSION,
        "stable-blabl",
        std::env::consts::OS,
        env!("CARGO_PKG_RUST_VERSION")
    )))
}
