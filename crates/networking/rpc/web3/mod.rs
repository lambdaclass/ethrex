use ethrex_storage::Store;
use serde_json::Value;

use crate::utils::{RpcErr, RpcRequest};

pub fn client_version(_req: &RpcRequest, _store: Store) -> Result<Value, RpcErr> {
    Ok(Value::String(format!(
        "{}/v{}-{}/{}/{}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        "stable-blabl",
        std::env::consts::OS,
        env!("CARGO_PKG_RUST_VERSION")
    )))
}
