use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct GetBadBlocksRequest;

impl RpcHandler for GetBadBlocksRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        if let Some(params) = params
            && !params.is_empty()
        {
            return Err(RpcErr::BadParams(format!(
                "Expected no params and {} were provided",
                params.len()
            )));
        }
        Ok(GetBadBlocksRequest)
    }

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        // Geth maintains a ring buffer of bad blocks encountered during chain
        // insertion. ethrex does not currently track bad blocks, so we return
        // an empty array which is valid (a healthy node has no bad blocks).
        Ok(Value::Array(vec![]))
    }
}
