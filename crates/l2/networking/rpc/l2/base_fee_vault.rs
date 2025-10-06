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
        let base_fee_vault_address = match &context.l1_ctx.blockchain.options.r#type {
            ethrex_blockchain::BlockchainType::L1 => None,
            ethrex_blockchain::BlockchainType::L2(l2_config) => {
                l2_config.fee_config.read().await.base_fee_vault
            }
        };

        Ok(
            serde_json::to_value(base_fee_vault_address.map(|addr| format!("{:#x}", addr)))
                .map_err(|e| {
                    ethrex_rpc::RpcErr::Internal(format!(
                        "Failed to serialize base fee vault address: {}",
                        e
                    ))
                })?,
        )
    }
}
