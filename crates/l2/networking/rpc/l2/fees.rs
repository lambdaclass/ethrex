use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

pub struct GetBaseFeeVaultAddress {
    pub block_number: u64,
}

impl RpcHandler for GetBaseFeeVaultAddress {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBaseFeeVaultAddress, RpcErr> {
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

        Ok(GetBaseFeeVaultAddress { block_number })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested BaseFeeVault with block number: {}",
            self.block_number
        );
        let base_fee_vault_address = context
            .rollup_store
            .get_fee_config_by_block(self.block_number)
            .await?
            .and_then(|fc| fc.base_fee_vault);

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

pub struct GetOperatorFeeVaultAddress {
    pub block_number: u64,
}

impl RpcHandler for GetOperatorFeeVaultAddress {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetOperatorFeeVaultAddress, RpcErr> {
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

        Ok(GetOperatorFeeVaultAddress { block_number })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested OperatorFeeVault with block number: {}",
            self.block_number
        );
        let operator_fee_config = context
            .rollup_store
            .get_fee_config_by_block(self.block_number)
            .await?
            .and_then(|fc| fc.operator_fee_config);

        Ok(serde_json::to_value(
            operator_fee_config.map(|config| format!("{:#x}", config.operator_fee_vault)),
        )
        .map_err(|e| {
            ethrex_rpc::RpcErr::Internal(format!(
                "Failed to serialize base fee vault address: {}",
                e
            ))
        })?)
    }
}

pub struct GetOperatorFee {
    pub block_number: u64,
}

impl RpcHandler for GetOperatorFee {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetOperatorFee, RpcErr> {
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

        Ok(GetOperatorFee { block_number })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested OperatorFee with block number: {}",
            self.block_number
        );
        let operator_fee_per_gas = context
            .rollup_store
            .get_fee_config_by_block(self.block_number)
            .await?
            .and_then(|fc| fc.operator_fee_config)
            .map(|config| config.operator_fee_per_gas)
            .unwrap_or(0);

        let operator_fee_hex = format!("0x{operator_fee_per_gas:x}");
        Ok(serde_json::Value::String(operator_fee_hex))
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
        debug!("Requested L1BlobBaseFee for block {}", self.block_number);

        let l1_blob_base_fee = context
            .rollup_store
            .get_fee_config_by_block(self.block_number)
            .await?
            .and_then(|fc| fc.l1_fee_config)
            .map(|cfg| cfg.l1_fee_per_blob_gas)
            .unwrap_or_default();

        serde_json::to_value(l1_blob_base_fee)
            .map_err(|err| RpcErr::Internal(format!("Failed to serialize L1BlobBaseFee: {}", err)))
    }
}
