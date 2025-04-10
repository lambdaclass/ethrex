use serde_json::Value;
use tracing::info;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::block_identifier::BlockIdentifier,
    utils::RpcErr,
};

#[derive(Debug)]
pub struct BatchByBlock {
    pub block: BlockIdentifier,
}

impl RpcHandler for BatchByBlock {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 params".to_owned()));
        };
        Ok(BatchByBlock {
            block: BlockIdentifier::parse(params[0].clone(), 0)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let l2_store = &context.l2_store;
        let storage = &context.storage;
        info!("Requested batch for block: {}", self.block);
        let block_number = match self.block.resolve_block_number(storage)? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        match l2_store.get_batch_number_for_block(block_number).await? {
            Some(batch_number) => serde_json::to_value(format!("{:#x}", batch_number))
                .map_err(|error| RpcErr::Internal(error.to_string())),
            None => Ok(Value::Null),
        }
    }
}
