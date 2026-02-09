use ethrex_rpc::types::block_identifier::BlockIdentifier;
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

pub struct GetBaseFeeVaultAddress {
    pub block: BlockIdentifier,
}

impl RpcHandler for GetBaseFeeVaultAddress {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBaseFeeVaultAddress, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 1 {
            return Err(ethrex_rpc::RpcErr::BadParams("Expected 1 param".to_owned()))?;
        };
        let block = BlockIdentifier::parse(params[0].clone(), 0)?;

        Ok(GetBaseFeeVaultAddress { block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested BaseFeeVault with block number: {}", self.block);
        let block_number = match self
            .block
            .resolve_block_number(&context.l1_ctx.storage)
            .await?
        {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let base_fee_vault_address = context
            .rollup_store
            .get_fee_config_by_block(block_number)
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
    pub block: BlockIdentifier,
}

impl RpcHandler for GetOperatorFeeVaultAddress {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetOperatorFeeVaultAddress, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 1 {
            return Err(ethrex_rpc::RpcErr::BadParams("Expected 1 param".to_owned()))?;
        };
        // Parse BlockNumber
        let block = BlockIdentifier::parse(params[0].clone(), 0)?;

        Ok(GetOperatorFeeVaultAddress { block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested OperatorFeeVault with block number: {}",
            self.block
        );
        let block_number = match self
            .block
            .resolve_block_number(&context.l1_ctx.storage)
            .await?
        {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let operator_fee_config = context
            .rollup_store
            .get_fee_config_by_block(block_number)
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
    pub block: BlockIdentifier,
}

impl RpcHandler for GetOperatorFee {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetOperatorFee, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 1 {
            return Err(ethrex_rpc::RpcErr::BadParams("Expected 1 param".to_owned()))?;
        };
        let block = BlockIdentifier::parse(params[0].clone(), 0)?;

        Ok(GetOperatorFee { block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested OperatorFee with block number: {}", self.block);
        let block_number = match self
            .block
            .resolve_block_number(&context.l1_ctx.storage)
            .await?
        {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let operator_fee_per_gas = context
            .rollup_store
            .get_fee_config_by_block(block_number)
            .await?
            .and_then(|fc| fc.operator_fee_config)
            .map(|config| config.operator_fee_per_gas)
            .unwrap_or(0);

        let operator_fee_hex = format!("0x{operator_fee_per_gas:x}");
        Ok(serde_json::Value::String(operator_fee_hex))
    }
}

pub struct GetL1FeeVaultAddress {
    pub block: BlockIdentifier,
}

impl RpcHandler for GetL1FeeVaultAddress {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetL1FeeVaultAddress, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 1 {
            return Err(ethrex_rpc::RpcErr::BadParams("Expected 1 param".to_owned()))?;
        };
        let block = BlockIdentifier::parse(params[0].clone(), 0)?;

        Ok(GetL1FeeVaultAddress { block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested L1FeeVault with block number: {}", self.block);
        let block_number = match self
            .block
            .resolve_block_number(&context.l1_ctx.storage)
            .await?
        {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let l1_fee_config = context
            .rollup_store
            .get_fee_config_by_block(block_number)
            .await?
            .and_then(|fc| fc.l1_fee_config);

        Ok(
            serde_json::to_value(l1_fee_config.map(|config| format!("{:#x}", config.l1_fee_vault)))
                .map_err(|e| {
                    ethrex_rpc::RpcErr::Internal(format!(
                        "Failed to serialize l1 fee vault address: {}",
                        e
                    ))
                })?,
        )
    }
}

pub struct GetL1BlobBaseFeeRequest {
    pub block: BlockIdentifier,
}

impl RpcHandler for GetL1BlobBaseFeeRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetL1BlobBaseFeeRequest, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 1 {
            return Err(ethrex_rpc::RpcErr::BadParams("Expected 1 param".to_owned()))?;
        };
        let block = BlockIdentifier::parse(params[0].clone(), 0)?;

        Ok(GetL1BlobBaseFeeRequest { block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested L1BlobBaseFee for block {}", self.block);
        let block_number = match self
            .block
            .resolve_block_number(&context.l1_ctx.storage)
            .await?
        {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };

        let l1_blob_base_fee = context
            .rollup_store
            .get_fee_config_by_block(block_number)
            .await?
            .and_then(|fc| fc.l1_fee_config)
            .map(|cfg| cfg.l1_fee_per_blob_gas)
            .unwrap_or_default();

        serde_json::to_value(l1_blob_base_fee)
            .map_err(|err| RpcErr::Internal(format!("Failed to serialize L1BlobBaseFee: {}", err)))
    }
}
