use ethrex_blockchain::find_parent_header;
use ethrex_rlp::encode::RLPEncode;
use serde_json::Value;
use tracing::info;

use crate::{
    types::{
        block::RpcBlock,
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        receipt::{RpcReceipt, RpcReceiptBlockInfo, RpcReceiptTxInfo},
    },
    utils::RpcErr,
    RpcApiContext, RpcHandler,
};
use ethrex_core::{
    types::{
        calculate_base_fee_per_blob_gas, Block, BlockBody, BlockHash, BlockHeader, BlockNumber,
        Receipt,
    },
    U256,
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
    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        info!("Requested block with number: {}", self.block);
        let block_number = match self.block.resolve_block_number(storage)? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = storage.get_block_header(block_number)?;
        let body = storage.get_block_body(block_number)?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            // Block not found
            _ => return Ok(Value::Null),
        };
        let hash = header.compute_block_hash();
        // TODO (#307): Remove TotalDifficulty.
        let total_difficulty = storage.get_block_total_difficulty(hash)?;
        let block = RpcBlock::build(
            header,
            body,
            hash,
            self.hydrated,
            total_difficulty.unwrap_or(U256::zero()),
        );

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
    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        info!("Requested block with hash: {:#x}", self.block);
        let block_number = match storage.get_block_number(self.block)? {
            Some(number) => number,
            _ => return Ok(Value::Null),
        };
        let header = storage.get_block_header(block_number)?;
        let body = storage.get_block_body(block_number)?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            // Block not found
            _ => return Ok(Value::Null),
        };
        let hash = header.compute_block_hash();
        // TODO (#307): Remove TotalDifficulty.
        let total_difficulty = storage.get_block_total_difficulty(hash)?;
        let block = RpcBlock::build(
            header,
            body,
            hash,
            self.hydrated,
            total_difficulty.unwrap_or(U256::zero()),
        );
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

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!(
            "Requested transaction count for block with number: {}",
            self.block
        );
        let block_number = match self.block.resolve_block_number(&context.storage)? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let block_body = match context.storage.get_block_body(block_number)? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let transaction_count = block_body.transactions.len();

        serde_json::to_value(format!("{:#x}", transaction_count))
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

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        info!("Requested receipts for block with number: {}", self.block);
        let block_number = match self.block.resolve_block_number(storage)? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = storage.get_block_header(block_number)?;
        let body = storage.get_block_body(block_number)?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            // Block not found
            _ => return Ok(Value::Null),
        };
        let receipts = get_all_block_rpc_receipts(block_number, header, body, storage)?;

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

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!(
            "Requested raw header for block with identifier: {}",
            self.block
        );
        let block_number = match self.block.resolve_block_number(&context.storage)? {
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

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!("Requested raw block: {}", self.block);
        let block_number = match self.block.resolve_block_number(&context.storage)? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = context.storage.get_block_header(block_number)?;
        let body = context.storage.get_block_body(block_number)?;
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

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        let block_number = match self.block.resolve_block_number(storage)? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = storage.get_block_header(block_number)?;
        let body = storage.get_block_body(block_number)?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            _ => return Ok(Value::Null),
        };
        let receipts: Vec<String> = get_all_block_receipts(block_number, header, body, storage)?
            .iter()
            .map(|receipt| format!("0x{}", hex::encode(receipt.encode_inner())))
            .collect();
        serde_json::to_value(receipts).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for BlockNumberRequest {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!("Requested latest block number");
        serde_json::to_value(format!("{:#x}", context.storage.get_latest_block_number()?))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetBlobBaseFee {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!("Requested blob gas price");
        let block_number = context.storage.get_latest_block_number()?;
        let header = match context.storage.get_block_header(block_number)? {
            Some(header) => header,
            _ => return Err(RpcErr::Internal("Could not get block header".to_owned())),
        };
        let parent_header = match find_parent_header(&header, &context.storage) {
            Ok(header) => header,
            Err(error) => return Err(RpcErr::Internal(error.to_string())),
        };
        let blob_base_fee =
            calculate_base_fee_per_blob_gas(parent_header.excess_blob_gas.unwrap_or_default());
        serde_json::to_value(format!("{:#x}", blob_base_fee))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub fn get_all_block_rpc_receipts(
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
    let blob_gas_price =
        calculate_base_fee_per_blob_gas(header.excess_blob_gas.unwrap_or_default());
    // Fetch receipt info from block
    let block_info = RpcReceiptBlockInfo::from_block_header(header);
    // Fetch receipt for each tx in the block and add block and tx info
    let mut last_cumulative_gas_used = 0;
    let mut current_log_index = 0;
    for (index, tx) in body.transactions.iter().enumerate() {
        let index = index as u64;
        let receipt = match storage.get_receipt(block_number, index)? {
            Some(receipt) => receipt,
            _ => return Err(RpcErr::Internal("Could not get receipt".to_owned())),
        };
        let gas_used = receipt.cumulative_gas_used - last_cumulative_gas_used;
        let tx_info =
            RpcReceiptTxInfo::from_transaction(tx.clone(), index, gas_used, blob_gas_price);
        let receipt = RpcReceipt::new(
            receipt.clone(),
            tx_info,
            block_info.clone(),
            current_log_index,
        );
        last_cumulative_gas_used += gas_used;
        current_log_index += receipt.logs.len() as u64;
        receipts.push(receipt);
    }
    Ok(receipts)
}

pub fn get_all_block_receipts(
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
        let receipt = match storage.get_receipt(block_number, index)? {
            Some(receipt) => receipt,
            _ => return Err(RpcErr::Internal("Could not get receipt".to_owned())),
        };
        receipts.push(receipt);
    }
    Ok(receipts)
}
