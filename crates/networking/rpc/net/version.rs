use serde_json::Value;

use crate::{context::RpcApiContext, server::RpcHandler, RpcErr};

pub struct Version;

impl RpcHandler for Version {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Version)
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let chain_spec = context.storage.get_chain_config()?;
        let value = serde_json::to_value(format!("{}", chain_spec.chain_id))?;
        Ok(value)
    }
}
