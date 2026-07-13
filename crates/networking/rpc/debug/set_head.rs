use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_common::types::BlockNumber;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

/// `debug_setHead`: rewinds the canonical head to the given block number.
///
/// Testing/debugging utility, mirroring geth's `debug_setHead`. Combined with
/// `testing_buildBlockV1` it lets a test reposition the head and build a new
/// block on top of it.
pub struct SetHeadRequest {
    pub block_number: BlockNumber,
}

impl RpcHandler for SetHeadRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected 1 param and {} were provided",
                params.len()
            )));
        }
        let block_number = match &params[0] {
            Value::String(hex) => {
                let trimmed = hex.strip_prefix("0x").unwrap_or(hex);
                BlockNumber::from_str_radix(trimmed, 16)
                    .map_err(|err| RpcErr::BadParams(format!("invalid block number: {err}")))?
            }
            Value::Number(num) => num
                .as_u64()
                .ok_or_else(|| RpcErr::BadParams("invalid block number".to_owned()))?,
            _ => return Err(RpcErr::BadParams("invalid block number".to_owned())),
        };
        Ok(Self { block_number })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block_hash = context
            .storage
            .get_canonical_block_hash(self.block_number)
            .await?
            .ok_or_else(|| {
                RpcErr::BadParams(format!(
                    "block {} not found in canonical chain",
                    self.block_number
                ))
            })?;

        // Point head, safe and finalized at the target block, rewinding the
        // canonical chain to it.
        // apply_fork_choice refuses to rewind below the stored finalized block
        // number. Surface this as BadParams so callers know they passed an
        // invalid target rather than interpreting it as a server bug.
        // `None`: use the finality-bounded default reorg cap (no operator override).
        apply_fork_choice(&context.storage, block_hash, block_hash, block_hash, None)
            .await
            .map_err(|err| RpcErr::BadParams(format!("cannot set head: {err}")))?;

        Ok(Value::Null)
    }
}
