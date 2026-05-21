use ethrex_common::H256;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct PreimageRequest {
    _hash: H256,
}

impl RpcHandler for PreimageRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected 1 param, got {}",
                params.len()
            )));
        }
        let hash: H256 = serde_json::from_value(params[0].clone())?;
        Ok(PreimageRequest { _hash: hash })
    }

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        // ethrex does not maintain a keccak preimage store.
        // Return null to indicate the preimage is not available.
        Ok(Value::Null)
    }
}
