pub mod transaction;

use crate::context::RpcApiContext;
use crate::rpc_types::RpcErr;
use crate::rpc_types::RpcRequest;
use crate::server::RpcHandler;
use serde_json::Value;

pub fn map_l2_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "ethrex_sendTransaction" => transaction::SponsoredTx::call(req, context),
        unknown_ethrex_l2_method => {
            Err(RpcErr::MethodNotFound(unknown_ethrex_l2_method.to_owned()))
        }
    }
}
