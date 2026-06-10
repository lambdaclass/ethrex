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
    Block, BlockBody, BlockHash, BlockHeader, Receipt, calculate_base_fee_per_blob_gas,
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
            // Block not found — including blocks whose body was pruned below
            // EarliestBlockNumber. A header-only response would misrepresent
            // a non-empty block as empty, so unavailable means null.
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
            // Block not found — including blocks whose body was pruned below
            // EarliestBlockNumber (see GetBlockByNumberRequest::handle).
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
        let receipts = get_all_block_rpc_receipts(header, body, storage, None).await?;

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
        let header = match storage.get_block_header(block_number)? {
            Some(header) => header,
            None => return Ok(Value::Null),
        };
        let receipts: Vec<String> = get_all_block_receipts(header, storage)
            .await?
            .iter()
            .map(|receipt| {
                format!(
                    "0x{}",
                    hex::encode(receipt.encode_inner_with_bloom(&ethrex_crypto::NativeCrypto))
                )
            })
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

/// Fetches RPC receipts for a block, optionally stopping after `target_index`.
///
/// When `target_index` is `Some(n)`, only receipts 0..=n are fetched using a
/// cursor pass — this is the fast path for `eth_getTransactionReceipt` which
/// only needs one receipt but requires preceding cumulative gas values.
///
/// When `target_index` is `None`, all receipts are fetched (for `eth_getBlockReceipts`).
pub async fn get_all_block_rpc_receipts(
    header: BlockHeader,
    body: BlockBody,
    storage: &Store,
    target_index: Option<u64>,
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
    let blob_base_fee_u64: u64 = blob_base_fee
        .try_into()
        .map_err(|_| RpcErr::Internal("blob_base_fee does not fit in u64".to_owned()))?;
    // Fetch receipt info from block
    let block_hash = header.hash();
    let block_info = RpcReceiptBlockInfo::from_block_header(header);
    // Fetch receipts: only up to target_index+1 when set, otherwise all
    let fetch_count = target_index
        .map(|ti| (ti + 1) as usize)
        .unwrap_or(body.transactions.len());
    let all_receipts = storage
        .get_receipts_for_block_from_index(&block_hash, 0, Some(fetch_count))
        .await?;
    // Return 500 on receipt count mismatch — this indicates data corruption
    // (missing receipts for a block that exists).
    if all_receipts.len() != fetch_count {
        return Err(RpcErr::Internal(format!(
            "Expected {} receipts, got {}",
            fetch_count,
            all_receipts.len()
        )));
    }
    let mut last_cumulative_gas_used = 0;
    let mut current_log_index = 0;
    for (index, (tx, receipt)) in body
        .transactions
        .iter()
        .zip(all_receipts.iter())
        .enumerate()
    {
        let index = index as u64;
        let gas_used = receipt.cumulative_gas_used - last_cumulative_gas_used;
        let tx_info = RpcReceiptTxInfo::from_transaction(
            tx.clone(),
            index,
            gas_used,
            blob_base_fee_u64,
            base_fee_per_gas,
        )?;
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

pub async fn get_all_block_receipts(
    header: BlockHeader,
    storage: &Store,
) -> Result<Vec<Receipt>, RpcErr> {
    // Check if this is the genesis block
    if header.parent_hash.is_zero() {
        return Ok(Vec::new());
    }
    let block_hash = header.hash();
    Ok(storage.get_receipts_for_block(&block_hash).await?)
}

#[cfg(test)]
mod pruning_rpc_tests {
    use super::*;
    use crate::{
        test_utils::{default_context_with_storage, setup_store},
        types::block_identifier::BlockIdentifier,
    };
    use ethrex_common::types::BlockNumber;

    // Helper: add a block with an empty body, make it canonical, then prune its body.
    // This simulates exactly the pruning scenario: canonical header exists, body is gone.
    async fn add_pruned_block(storage: &Store, block_number: BlockNumber) -> BlockHash {
        let header = BlockHeader {
            number: block_number,
            ..Default::default()
        };
        let block = Block::new(
            header,
            BlockBody {
                transactions: vec![],
                ommers: vec![],
                withdrawals: Some(vec![]),
            },
        );
        let hash = block.hash();
        storage.add_block(block).await.unwrap();
        storage
            .forkchoice_update(vec![], block_number, hash, None, None)
            .await
            .unwrap();
        storage.prune_block_height(block_number).await.unwrap();
        hash
    }

    /// When a block body has been pruned (canonical header exists, body is
    /// None), the RPC handler must return null — same as an unknown block.
    /// We deliberately don't synthesize a header-only response: the real
    /// header (non-empty transactionsRoot/gasUsed) next to an empty
    /// transactions list would misrepresent a non-empty block as empty.
    #[tokio::test]
    async fn get_block_by_number_returns_null_when_body_pruned() {
        let storage = setup_store().await;
        add_pruned_block(&storage, 5).await;

        // --- include_full_tx = true ---
        let req_full = GetBlockByNumberRequest {
            block: BlockIdentifier::Number(5),
            hydrated: true,
        };
        let context = default_context_with_storage(storage.clone()).await;
        let result = req_full.handle(context).await.unwrap();
        assert_eq!(result, Value::Null, "expected null for pruned block");

        // --- include_full_tx = false (hashes only) ---
        let req_hashes = GetBlockByNumberRequest {
            block: BlockIdentifier::Number(5),
            hydrated: false,
        };
        let context2 = default_context_with_storage(storage).await;
        let result_hashes = req_hashes.handle(context2).await.unwrap();
        assert_eq!(
            result_hashes,
            Value::Null,
            "expected null for pruned block (hash mode)"
        );
    }

    /// A block at or above `earliest_block_number` with a missing body returns null
    /// (it's genuinely absent, not pruned — canonical header doesn't exist either).
    #[tokio::test]
    async fn get_block_by_number_returns_null_when_block_absent_above_earliest() {
        let storage = setup_store().await;

        // earliest = 6; block 10 has no header or body at all.
        storage.update_earliest_block_number(6).await.unwrap();

        let context = default_context_with_storage(storage).await;
        let req = GetBlockByNumberRequest {
            block: BlockIdentifier::Number(10),
            hydrated: true,
        };
        let result = req.handle(context).await.unwrap();
        assert_eq!(
            result,
            Value::Null,
            "expected null for non-pruned missing body"
        );
    }

    /// Same pruning behaviour for getBlockByHash.
    #[tokio::test]
    async fn get_block_by_hash_returns_null_when_body_pruned() {
        let storage = setup_store().await;
        let hash = add_pruned_block(&storage, 7).await;

        let context = default_context_with_storage(storage).await;

        let req = GetBlockByHashRequest {
            block: hash,
            hydrated: true,
        };
        let result = req.handle(context).await.unwrap();
        assert_eq!(
            result,
            Value::Null,
            "expected null for pruned block (by hash)"
        );
    }

    /// After pruning a block, `eth_getBlockReceipts` must return null because the
    /// block body (needed to enumerate transactions and receipts) is gone.
    #[tokio::test]
    async fn get_block_receipts_returns_null_for_pruned_height() {
        use crate::types::block_identifier::BlockIdentifierOrHash;

        let storage = setup_store().await;
        let hash = add_pruned_block(&storage, 9).await;
        let _ = hash; // body is pruned; canonical header still exists

        // earliest = 9, so block 9 is at the pruning horizon (body missing).
        storage.update_earliest_block_number(9).await.unwrap();

        let req = GetBlockReceiptsRequest {
            block: BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(9)),
        };
        let context = default_context_with_storage(storage).await;
        let result = req.handle(context).await.unwrap();
        assert_eq!(
            result,
            serde_json::Value::Null,
            "expected null for receipts of pruned block"
        );
    }
}
