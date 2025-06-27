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
        Ok(GetBatchByBatchNumberRequest {
            batch_number: serde_json::from_value(params[0].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let rollup_storage = &context.rollup_store;
        info!("Requested batch with number: {}", self.batch_number);
        let batch = rollup_storage.get_batch(self.batch_number).await?;

        // TODO: handle the case where the batch is not found
        // TODO: implement batch serialization

        serde_json::to_value(&batch).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
