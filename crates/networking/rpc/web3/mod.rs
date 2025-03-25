pub mod client;

use serde_json::Value;

use crate::context::RpcApiContext;
use crate::errors::RpcErr;
use crate::router::RpcHandler;
use crate::rpc_types::RpcRequest;

pub fn map_web3_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "web3_clientVersion" => client::ClientVersion::call(req, context),
        unknown_web3_method => Err(RpcErr::MethodNotFound(unknown_web3_method.to_owned())),
    }
}
