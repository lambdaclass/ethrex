use serde_json::Value;
use tracing::info;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

pub struct GetBatchByBatchNumberRequest {
    pub batch_number: u64,
}

impl RpcHandler for GetBatchByBatchNumberRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBatchByBatchNumberRequest, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 2 {
            return Err(ethrex_rpc::RpcErr::BadParams(
                "Expected 2 params".to_owned(),
            ))?;
        };
        // Parse BatchNumber
        let hex_str = serde_json::from_value::<String>(params[0].clone())
            .map_err(|e| ethrex_rpc::RpcErr::BadParams(e.to_string()))?;

        // Check that the BatchNumber is 0x prefixed
        let hex_str = hex_str
            .strip_prefix("0x")
            .ok_or(ethrex_rpc::RpcErr::BadHexFormat(0))?;

        // Parse hex string
        let batch_number =
            u64::from_str_radix(hex_str, 16).map_err(|_| ethrex_rpc::RpcErr::BadHexFormat(0))?;

        Ok(GetBatchByBatchNumberRequest { batch_number })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let rollup_storage = &context.rollup_store;
        info!("Requested batch with number: {}", self.batch_number);
        let Some(batch) = rollup_storage.get_batch(self.batch_number).await? else {
            return Ok(Value::Null);
        };

        serde_json::to_value(&batch).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
