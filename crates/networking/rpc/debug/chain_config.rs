use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct ChainConfigRequest;

impl RpcHandler for ChainConfigRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        if let Some(params) = params {
            if !params.is_empty() {
                return Err(RpcErr::BadParams(format!(
                    "Expected no params and {} were provided",
                    params.len()
                )));
            }
        }
        Ok(ChainConfigRequest)
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let chain_config = context.storage.get_chain_config();
        serde_json::to_value(chain_config).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}
