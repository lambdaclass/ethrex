use crate::rpc::RpcHandler;

pub struct GetFeeVaultAddress;

impl RpcHandler for GetFeeVaultAddress {
    fn parse(_params: &Option<Vec<serde_json::Value>>) -> Result<Self, ethrex_rpc::RpcErr> {
        Ok(GetFeeVaultAddress)
    }

    async fn handle(
        &self,
        context: ethrex_rpc::RpcApiContext,
    ) -> Result<serde_json::Value, ethrex_rpc::RpcErr> {
        let fee_vault_address = context
            .l1_ctx
            .fee_vault_address
            .map(|addr| format!("{:#x}", addr));
        Ok(serde_json::to_value(fee_vault_address).map_err(|e| {
            ethrex_rpc::RpcErr::Internal(format!("Failed to serialize fee vault address: {}", e))
        })?)
    }
}
