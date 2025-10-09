use serde_json::Value;
use tracing::debug;

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

pub struct GetL1BlobBaseFeeRequest {
    pub block_number: u64,
}

impl RpcHandler for GetL1BlobBaseFeeRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetL1BlobBaseFeeRequest, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 1 {
            return Err(ethrex_rpc::RpcErr::BadParams(
                "Expected 1 params".to_owned(),
            ))?;
        };
        // Parse BlockNumber
        let hex_str = serde_json::from_value::<String>(params[0].clone())
            .map_err(|e| ethrex_rpc::RpcErr::BadParams(e.to_string()))?;

        // Check that the BlockNumber is 0x prefixed
        let hex_str = hex_str
            .strip_prefix("0x")
            .ok_or(ethrex_rpc::RpcErr::BadHexFormat(0))?;

        // Parse hex string
        let block_number =
            u64::from_str_radix(hex_str, 16).map_err(|_| ethrex_rpc::RpcErr::BadHexFormat(0))?;

        Ok(GetL1BlobBaseFeeRequest { block_number })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested L1BlobBaseFee with block number: {}",
            self.block_number
        );
        let Some(l1_blob_base_fee) = context
            .rollup_store
            .get_l1_blob_base_fee_by_block(self.block_number)
            .await?
        else {
            return Ok(Value::Number(0.into()));
        };

        serde_json::to_value(l1_blob_base_fee).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
