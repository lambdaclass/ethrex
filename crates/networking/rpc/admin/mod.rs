pub mod node_info;

use serde_json::Value;

use crate::context::RpcApiContext;
use crate::rpc_types::RpcErr;
use crate::rpc_types::RpcRequest;
use crate::server::RpcHandler;

pub fn map_admin_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "admin_nodeInfo" => node_info::NodeInfoRequest::call(req, context),
        unknown_admin_method => Err(RpcErr::MethodNotFound(unknown_admin_method.to_owned())),
    }
}
