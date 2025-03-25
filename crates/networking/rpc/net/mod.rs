pub mod version;

use serde_json::Value;

use crate::{context::RpcApiContext, rpc_types::RpcRequest, server::RpcHandler, RpcErr};

pub fn map_net_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "net_version" => version::Version::call(req, context),
        unknown_net_method => Err(RpcErr::MethodNotFound(unknown_net_method.to_owned())),
    }
}
