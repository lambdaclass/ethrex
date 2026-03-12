use crate::rpc::{RpcApiContext, RpcHandler};
use crate::utils::RpcErr;
use serde::Serialize;
use serde_json::Value;
use tracing::debug;

/// Response for the `ethrex_metadata` RPC method.
/// Returns basic chain metadata that the L2 node can self-describe.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataResponse {
    chain_id: String,
    latest_batch: Option<String>,
}

pub struct MetadataRequest;

impl RpcHandler for MetadataRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        if params.as_ref().is_some_and(|p| !p.is_empty()) {
            return Err(ethrex_rpc::RpcErr::BadParams(
                "Expected 0 params".to_owned(),
            ))?;
        }
        Ok(Self)
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested chain metadata");

        let chain_config = context.l1_ctx.storage.get_chain_config();
        let chain_id = chain_config.chain_id;

        let latest_batch = context
            .rollup_store
            .get_batch_number()
            .await?
            .map(|n| format!("{n:#x}"));

        let response = MetadataResponse {
            chain_id: format!("{chain_id:#x}"),
            latest_batch,
        };

        serde_json::to_value(response).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}
