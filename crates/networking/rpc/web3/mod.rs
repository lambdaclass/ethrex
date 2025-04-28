use ethrex_storage::Store;
use serde_json::Value;

use crate::utils::{RpcErr, RpcRequest};

pub fn client_version(
    _req: &RpcRequest,
    _store: Store,
    client_version: String,
) -> Result<Value, RpcErr> {
    Ok(Value::String(client_version))
}
