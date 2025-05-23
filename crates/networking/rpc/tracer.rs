use keccak_hash::H256;

use crate::{rpc::RpcHandler, utils::RpcErr};

pub struct TraceTransactionRequest {
    tx_hash: H256,
}
// TODO: add opts
// make callTracer default, fail if other is received

impl RpcHandler for TraceTransactionRequest {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 params".to_owned()));
        };
        Ok(TraceTransactionRequest {
            tx_hash: serde_json::from_value(params[0].clone())?,
        })
    }

    async fn handle(
        &self,
        context: crate::rpc::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let tx = context
            .storage
            .get_transaction_by_hash(self.tx_hash)
            .await?;
        let tx = context.storage.get_transaction_location(tx).await?;

        todo!()
    }
}
