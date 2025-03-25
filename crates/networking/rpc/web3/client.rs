use serde_json::Value;

use crate::context::RpcApiContext;
use crate::rpc_types::RpcErr;
use crate::server::RpcHandler;

pub struct ClientVersion;

impl RpcHandler for ClientVersion {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(ClientVersion)
    }

    fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        Ok(Value::String("ethrex@0.1.0".to_owned()))
    }
}
