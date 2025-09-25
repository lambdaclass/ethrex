use serde_json::Value;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

pub struct GetFeeVaultAddress;

impl RpcHandler for GetFeeVaultAddress {
    fn parse(_params: &Option<Vec<Value>>) -> Result<GetFeeVaultAddress, RpcErr> {
        Ok(GetFeeVaultAddress)
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let fee_vault_address = context
            .l1_ctx
            .blockchain
            .fee_vault
            .map(|addr| format!("{:#x}", addr));
        Ok(serde_json::to_value(fee_vault_address).map_err(|e| {
            ethrex_rpc::RpcErr::Internal(format!("Failed to serialize fee vault address: {}", e))
        })?)
    }
}
