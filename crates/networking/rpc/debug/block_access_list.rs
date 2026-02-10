use ethrex_rlp::encode::RLPEncode;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifier};

pub struct BlockAccessListRequest {
    pub block_id: BlockIdentifier,
}

impl RpcHandler for BlockAccessListRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        }
        let block_id = BlockIdentifier::parse(params[0].clone(), 0)?;
        Ok(BlockAccessListRequest { block_id })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // Resolve block number
        let block_number = self
            .block_id
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::Internal(
                "Failed to resolve block number".to_string(),
            ))?;

        // Get block header and body
        let header = context
            .storage
            .get_block_header(block_number)?
            .ok_or(RpcErr::Internal("Block header not found".to_string()))?;
        let block = context
            .storage
            .get_block_by_hash(header.hash())
            .await?
            .ok_or(RpcErr::Internal("Block not found".to_string()))?;

        // Generate BAL by re-executing
        let bal = context
            .blockchain
            .generate_bal_for_block(&block)
            .map_err(|e| RpcErr::Internal(format!("Failed to generate BAL: {e}")))?;

        // Return BAL as RLP hex string (null for pre-Amsterdam blocks)
        match bal {
            Some(bal) => {
                let rlp_bytes = bal.encode_to_vec();
                Ok(Value::String(format!("0x{}", hex::encode(rlp_bytes))))
            }
            None => Ok(Value::Null),
        }
    }
}
