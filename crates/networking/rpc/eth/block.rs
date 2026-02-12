use ethrex_rlp::encode::RLPEncode;
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block::RpcBlock,
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        receipt::{RpcReceipt, RpcReceiptBlockInfo, RpcReceiptTxInfo},
    },
    utils::RpcErr,
};
use ethrex_common::types::{
    Block, BlockBody, BlockHash, BlockHeader, BlockNumber, Receipt, calculate_base_fee_per_blob_gas,
};
use ethrex_storage::Store;

pub struct GetBlockByNumberRequest {
    pub block: BlockIdentifier,
    pub hydrated: bool,
}

pub struct GetBlockByHashRequest {
    pub block: BlockHash,
    pub hydrated: bool,
}

pub struct GetBlockTransactionCountRequest {
    pub block: BlockIdentifierOrHash,
}

pub struct GetBlockReceiptsRequest {
    pub block: BlockIdentifierOrHash,
}

#[derive(Clone, Debug)]
pub struct GetRawHeaderRequest {
    pub block: BlockIdentifier,
}

pub struct GetRawBlockRequest {
    pub block: BlockIdentifier,
}

pub struct GetRawReceipts {
    pub block: BlockIdentifier,
}

pub struct BlockNumberRequest;
pub struct GetBlobBaseFee;

impl RpcHandler for GetBlockByNumberRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBlockByNumberRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetBlockByNumberRequest {
            block: BlockIdentifier::parse(params[0].clone(), 0)?,
            hydrated: serde_json::from_value(params[1].clone())?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!("Requested block with number: {}", self.block);
        let block_number = match self.block.resolve_block_number(storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = storage.get_block_header(block_number)?;
        let body = storage.get_block_body(block_number).await?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            // Block not found
            _ => return Ok(Value::Null),
        };
        let hash = header.hash();
        let block = RpcBlock::build(header, body, hash, self.hydrated)?;

        serde_json::to_value(&block).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetBlockByHashRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBlockByHashRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetBlockByHashRequest {
            block: serde_json::from_value(params[0].clone())?,
            hydrated: serde_json::from_value(params[1].clone())?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!("Requested block with hash: {:#x}", self.block);
        let block_number = match storage.get_block_number(self.block).await? {
            Some(number) => number,
            _ => return Ok(Value::Null),
        };
        let header = storage.get_block_header(block_number)?;
        let body = storage.get_block_body(block_number).await?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            // Block not found
            _ => return Ok(Value::Null),
        };
        let hash = header.hash();
        let block = RpcBlock::build(header, body, hash, self.hydrated)?;
        serde_json::to_value(&block).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetBlockTransactionCountRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBlockTransactionCountRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetBlockTransactionCountRequest {
            block: BlockIdentifierOrHash::parse(params[0].clone(), 0)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested transaction count for block with number: {}",
            self.block
        );
        let block_number = match self.block.resolve_block_number(&context.storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let block_body = match context.storage.get_block_body(block_number).await? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let transaction_count = block_body.transactions.len();

        serde_json::to_value(format!("{transaction_count:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetBlockReceiptsRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBlockReceiptsRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetBlockReceiptsRequest {
            block: BlockIdentifierOrHash::parse(params[0].clone(), 0)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!("Requested receipts for block with number: {}", self.block);
        let block_number = match self.block.resolve_block_number(storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = storage.get_block_header(block_number)?;
        let body = storage.get_block_body(block_number).await?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            // Block not found
            _ => return Ok(Value::Null),
        };
        let receipts = get_all_block_rpc_receipts(block_number, header, body, storage).await?;

        serde_json::to_value(&receipts).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetRawHeaderRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetRawHeaderRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetRawHeaderRequest {
            block: BlockIdentifier::parse(params[0].clone(), 0)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested raw header for block with identifier: {}",
            self.block
        );
        let block_number = match self.block.resolve_block_number(&context.storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = context
            .storage
            .get_block_header(block_number)?
            .ok_or(RpcErr::BadParams("Header not found".to_owned()))?;

        let str_encoded = format!("0x{}", hex::encode(header.encode_to_vec()));
        Ok(Value::String(str_encoded))
    }
}

impl RpcHandler for GetRawBlockRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetRawBlockRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };

        Ok(GetRawBlockRequest {
            block: BlockIdentifier::parse(params[0].clone(), 0)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested raw block: {}", self.block);
        let block_number = match self.block.resolve_block_number(&context.storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = context.storage.get_block_header(block_number)?;
        let body = context.storage.get_block_body(block_number).await?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            _ => return Ok(Value::Null),
        };
        let block = Block::new(header, body).encode_to_vec();

        serde_json::to_value(format!("0x{}", &hex::encode(block)))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetRawReceipts {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };

        Ok(GetRawReceipts {
            block: BlockIdentifier::parse(params[0].clone(), 0)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        let block_number = match self.block.resolve_block_number(storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = storage.get_block_header(block_number)?;
        let body = storage.get_block_body(block_number).await?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            _ => return Ok(Value::Null),
        };
        let receipts: Vec<String> = get_all_block_receipts(block_number, header, body, storage)
            .await?
            .iter()
            .map(|receipt| format!("0x{}", hex::encode(receipt.encode_inner_with_bloom())))
            .collect();
        serde_json::to_value(receipts).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for BlockNumberRequest {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested latest block number");
        serde_json::to_value(format!(
            "{:#x}",
            context.storage.get_latest_block_number().await?
        ))
        .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetBlobBaseFee {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested blob gas price");
        let block_number = context.storage.get_latest_block_number().await?;
        let header = match context.storage.get_block_header(block_number)? {
            Some(header) => header,
            _ => return Err(RpcErr::Internal("Could not get block header".to_owned())),
        };
        let config = context.storage.get_chain_config();
        let blob_base_fee = calculate_base_fee_per_blob_gas(
            header.excess_blob_gas.unwrap_or_default(),
            config
                .get_fork_blob_schedule(header.timestamp)
                .map(|schedule| schedule.base_fee_update_fraction)
                .unwrap_or_default(),
        );

        serde_json::to_value(format!("{blob_base_fee:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub async fn get_all_block_rpc_receipts(
    block_number: BlockNumber,
    header: BlockHeader,
    body: BlockBody,
    storage: &Store,
) -> Result<Vec<RpcReceipt>, RpcErr> {
    let mut receipts = Vec::new();
    // Check if this is the genesis block
    if header.parent_hash.is_zero() {
        return Ok(receipts);
    }
    let config = storage.get_chain_config();
    let blob_base_fee = calculate_base_fee_per_blob_gas(
        header.excess_blob_gas.unwrap_or_default(),
        config
            .get_fork_blob_schedule(header.timestamp)
            .map(|schedule| schedule.base_fee_update_fraction)
            .unwrap_or_default(),
    );
    let base_fee_per_gas = header.base_fee_per_gas;
    // Fetch receipt info from block
    let block_info = RpcReceiptBlockInfo::from_block_header(header);
    // Fetch receipt for each tx in the block and add block and tx info
    let mut last_cumulative_gas_used = 0;
    let mut current_log_index = 0;
    for (index, tx) in body.transactions.iter().enumerate() {
        let index = index as u64;
        let receipt = match storage.get_receipt(block_number, index).await? {
            Some(receipt) => receipt,
            _ => return Err(RpcErr::Internal("Could not get receipt".to_owned())),
        };
        let gas_used = receipt.cumulative_gas_used - last_cumulative_gas_used;
        let tx_info = RpcReceiptTxInfo::from_transaction(
            tx,
            index,
            gas_used,
            blob_base_fee,
            base_fee_per_gas,
        )?;
        let receipt = RpcReceipt::new(receipt, tx_info, block_info.clone(), current_log_index);
        last_cumulative_gas_used += gas_used;
        current_log_index += receipt.logs.len() as u64;
        receipts.push(receipt);
    }
    Ok(receipts)
}

pub async fn get_all_block_receipts(
    block_number: BlockNumber,
    header: BlockHeader,
    body: BlockBody,
    storage: &Store,
) -> Result<Vec<Receipt>, RpcErr> {
    let mut receipts = Vec::new();
    // Check if this is the genesis block
    if header.parent_hash.is_zero() {
        return Ok(receipts);
    }
    for (index, _) in body.transactions.iter().enumerate() {
        let index = index as u64;
        let receipt = match storage.get_receipt(block_number, index).await? {
            Some(receipt) => receipt,
            _ => return Err(RpcErr::Internal("Could not get receipt".to_owned())),
        };
        receipts.push(receipt);
    }
    Ok(receipts)
}
