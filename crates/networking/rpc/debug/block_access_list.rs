use crate::types::block_identifier::BlockIdentifier;
use crate::{RpcErr, RpcHandler};

pub struct GetBlockAccessListRequest {
    number: BlockIdentifier,
}

impl RpcHandler for GetBlockAccessListRequest {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, crate::RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        }
        let number = BlockIdentifier::parse(params[0].clone(), 0)?;
        Ok(Self { number })
    }

    async fn handle(
        &self,
        context: crate::RpcApiContext,
    ) -> Result<serde_json::Value, crate::RpcErr> {
        let block = self
            .number
            .resolve_block(&context.storage)
            .await?
            .ok_or(RpcErr::Internal("Block not found".to_owned()))?;
        let bal_response = context
            .blockchain
            .trace_block_access_list(block)
            .await
            .map_err(|err| RpcErr::Internal(err.to_string()))?;
        serde_json::to_value(bal_response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
