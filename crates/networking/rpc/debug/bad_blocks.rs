use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use serde::Serialize;
use serde_json::Value;
use tracing::debug;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block::RpcBlock};

pub struct GetBadBlocksRequest;

/// A single entry returned by `debug_getBadBlocks`, mirroring geth's `BadBlockArgs`.
#[derive(Serialize)]
struct BadBlock {
    hash: H256,
    block: RpcBlock,
    rlp: String,
}

impl RpcHandler for GetBadBlocksRequest {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(GetBadBlocksRequest)
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested bad blocks");

        let bad_blocks = context.storage.get_bad_blocks().await?;
        let mut results = Vec::with_capacity(bad_blocks.len());
        for block in bad_blocks {
            let hash = block.hash();
            let rlp = format!("0x{}", hex::encode(block.encode_to_vec()));
            let rpc_block = RpcBlock::build(block.header, block.body, hash, true)?;
            results.push(BadBlock {
                hash,
                block: rpc_block,
                rlp,
            });
        }

        serde_json::to_value(results).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
