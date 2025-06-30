use crate::rpc::RpcHandler;

pub struct NodeStatus;

impl RpcHandler for NodeStatus {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, crate::utils::RpcErr> {
        if params.is_some() {
            return Err(ethrex_rpc::RpcErr::BadParams(
                "NodeStatus does not accept parameters".to_owned(),
            )
            .into());
        }

        Ok(NodeStatus {})
    }

    async fn handle(
        &self,
        context: crate::rpc::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        serde_json::to_value(context.sequencer_state.status().await)
            .map_err(|e| ethrex_rpc::RpcErr::Internal(e.to_string()).into())
    }
}
