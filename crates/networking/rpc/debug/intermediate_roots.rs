use std::time::Duration;

use ethrex_common::{H256, serde_utils};
use serde::Deserialize;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

const DEFAULT_REEXEC: u32 = 128;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct IntermediateRootsRequest {
    block_hash: H256,
    timeout: Duration,
    reexec: u32,
}

#[derive(Deserialize, Default)]
struct IntermediateRootsConfig {
    #[serde(default, with = "serde_utils::duration::opt")]
    timeout: Option<Duration>,
    #[serde(default)]
    reexec: Option<u32>,
}

impl RpcHandler for IntermediateRootsRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() || params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected 1-2 params, got {}",
                params.len()
            )));
        }
        let block_hash: H256 = serde_json::from_value(params[0].clone())?;
        let config: IntermediateRootsConfig = if params.len() > 1 {
            serde_json::from_value(params[1].clone())?
        } else {
            IntermediateRootsConfig::default()
        };
        Ok(IntermediateRootsRequest {
            block_hash,
            timeout: config.timeout.unwrap_or(DEFAULT_TIMEOUT),
            reexec: config.reexec.unwrap_or(DEFAULT_REEXEC),
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block = context
            .storage
            .get_block_by_hash(self.block_hash)
            .await?
            .ok_or(RpcErr::Internal("Block not found".to_string()))?;

        let roots = context
            .blockchain
            .compute_intermediate_roots(block, self.reexec, self.timeout)
            .await
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        Ok(serde_json::to_value(roots)?)
    }
}
