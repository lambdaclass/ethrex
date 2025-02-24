use crate::RpcHandler;
use serde::{Deserialize, Serialize};
use tree_hash_derive::TreeHash;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TreeHash)]
#[serde(rename_all = "camelCase")]
pub struct FragV0;

impl RpcHandler for FragV0 {
    fn parse(_params: &Option<Vec<serde_json::Value>>) -> Result<Self, crate::utils::RpcErr> {
        tracing::info!("parsing frag");
        Ok(Self)
    }

    fn handle(
        &self,
        _context: crate::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        tracing::info!("handling frag");
        Ok(serde_json::Value::Null)
    }
}
