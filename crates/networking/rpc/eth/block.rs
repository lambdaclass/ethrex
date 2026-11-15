use ethrex_rlp::encode::RLPEncode;
use serde_json::{Value, value::RawValue};
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block::{RpcBlock, RpcHeader},
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash, BlockTag},
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

pub struct GetHeaderByNumberRequest {
    pub block: BlockIdentifier,
}

pub struct GetHeaderByHashRequest {
    pub block: BlockHash,
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

/// `eth_getUncleCountByBlockHash` / `eth_getUncleCountByBlockNumber`.
pub struct GetUncleCountRequest {
    pub block: BlockIdentifierOrHash,
}

pub struct BlockNumberRequest;
pub struct GetBlobBaseFee;

impl RpcHandler for GetBlockByNumberRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetBlockByNumberRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetBlockByNumberRequest {
            block: BlockIdentifier::parse_param(&params[0], 0)?,
            hydrated: serde_json::from_str(params[1].get())?,
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
        let body = get_block_body(storage, block_number).await?;
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
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetBlockByHashRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetBlockByHashRequest {
            block: serde_json::from_str(params[0].get())?,
            hydrated: serde_json::from_str(params[1].get())?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!("Requested block with hash: {:#x}", self.block);
        let Some(header) = storage.get_block_header_by_hash(self.block)? else {
            return Ok(Value::Null);
        };
        let body = match storage.get_block_body_by_hash(self.block).await? {
            Some(body) => Some(body),
            None => {
                ensure_history_retained(storage, header.number).await?;
                // As in `get_block_body`.
                storage.get_block_body_by_hash(self.block).await?
            }
        };
        let Some(body) = body else {
            return Ok(Value::Null);
        };
        let block = RpcBlock::build(header, body, self.block, self.hydrated)?;
        serde_json::to_value(&block).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetHeaderByNumberRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetHeaderByNumberRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetHeaderByNumberRequest {
            block: BlockIdentifier::parse_param(&params[0], 0)?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!("Requested header with number: {}", self.block);
        // Per the spec, the pending tag returns null instead of aliasing latest.
        if matches!(self.block, BlockIdentifier::Tag(BlockTag::Pending)) {
            return Ok(Value::Null);
        }
        let block_number = match self.block.resolve_block_number(storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = match storage.get_block_header(block_number)? {
            Some(header) => header,
            _ => return Ok(Value::Null),
        };
        let hash = header.hash();
        serde_json::to_value(RpcHeader { hash, header })
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetHeaderByHashRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetHeaderByHashRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetHeaderByHashRequest {
            block: serde_json::from_str(params[0].get())?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!("Requested header with hash: {:#x}", self.block);
        let Some(header) = storage.get_block_header_by_hash(self.block)? else {
            return Ok(Value::Null);
        };
        serde_json::to_value(RpcHeader {
            hash: self.block,
            header,
        })
        .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetBlockTransactionCountRequest {
    fn parse(
        params: &Option<Vec<Box<RawValue>>>,
    ) -> Result<GetBlockTransactionCountRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetBlockTransactionCountRequest {
            block: BlockIdentifierOrHash::parse_param(&params[0], 0)?,
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
        let block_body = match get_block_body(&context.storage, block_number).await? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let transaction_count = block_body.transactions.len();

        serde_json::to_value(format!("{transaction_count:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetUncleCountRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetUncleCountRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetUncleCountRequest {
            block: BlockIdentifierOrHash::parse_param(&params[0], 0)?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested uncle count for block: {}", self.block);
        // Post-merge every canonical block has an empty ommers list, so this is
        // always `0x0` in practice. It is still derived from the stored body
        // rather than hardcoded, so pre-merge history read from a synced archive
        // answers truthfully. An unknown block yields `null`, matching the
        // transaction-count getters and the other clients.
        let block_number = match self.block.resolve_block_number(&context.storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let block_body = match get_block_body(&context.storage, block_number).await? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let uncle_count = block_body.ommers.len();

        serde_json::to_value(format!("{uncle_count:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetBlockReceiptsRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetBlockReceiptsRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetBlockReceiptsRequest {
            block: BlockIdentifierOrHash::parse_param(&params[0], 0)?,
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
        let body = get_block_body(storage, block_number).await?;
        let (header, body) = match (header, body) {
            (Some(header), Some(body)) => (header, body),
            // Block not found
            _ => return Ok(Value::Null),
        };
        let receipts = get_all_block_rpc_receipts(header, body, storage).await?;

        serde_json::to_value(&receipts).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetRawHeaderRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetRawHeaderRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(GetRawHeaderRequest {
            block: BlockIdentifier::parse_param(&params[0], 0)?,
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
            .ok_or(RpcErr::ServerError("Header not found".to_owned()))?;

        let str_encoded = format!("0x{}", hex::encode(header.encode_to_vec()));
        Ok(Value::String(str_encoded))
    }
}

impl RpcHandler for GetRawBlockRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetRawBlockRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };

        Ok(GetRawBlockRequest {
            block: BlockIdentifier::parse_param(&params[0], 0)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested raw block: {}", self.block);
        let block_number = match self.block.resolve_block_number(&context.storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = context.storage.get_block_header(block_number)?;
        let body = get_block_body(&context.storage, block_number).await?;
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
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };

        Ok(GetRawReceipts {
            block: BlockIdentifier::parse_param(&params[0], 0)?,
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
    fn parse(_params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested latest block number");
        serde_json::to_value(format!("{:#x}", context.storage.get_latest_block_number()?))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetBlobBaseFee {
    fn parse(_params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested blob gas price");
        let block_number = context.storage.get_latest_block_number()?;
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

/// `4444` for a block below the earliest one whose body and receipts are kept.
pub(crate) async fn ensure_history_retained(
    storage: &Store,
    number: BlockNumber,
) -> Result<(), RpcErr> {
    ensure_retained(storage, number, false).await
}

/// Also `4444` at the earliest block: a snap pivot is stored without its receipts.
pub(crate) async fn ensure_receipts_retained(
    storage: &Store,
    number: BlockNumber,
) -> Result<(), RpcErr> {
    ensure_retained(storage, number, true).await
}

async fn ensure_retained(
    storage: &Store,
    number: BlockNumber,
    including_earliest: bool,
) -> Result<(), RpcErr> {
    // Never stored above the head.
    if number > storage.get_latest_block_number()? {
        return Ok(());
    }
    let earliest = storage.get_earliest_block_number().await?;
    if number < earliest {
        return Err(pruned_history(number, earliest));
    }
    if including_earliest && number == earliest {
        return Err(RpcErr::PrunedHistoryUnavailable(format!(
            "receipts of block {number} are unavailable, receipts are available from block {}",
            number + 1
        )));
    }
    Ok(())
}

pub(crate) fn pruned_history(number: BlockNumber, earliest: BlockNumber) -> RpcErr {
    RpcErr::PrunedHistoryUnavailable(format!(
        "history of block {number} is unavailable, the earliest block with history is {earliest}"
    ))
}

pub(crate) async fn get_block_body(
    storage: &Store,
    number: BlockNumber,
) -> Result<Option<BlockBody>, RpcErr> {
    if let Some(body) = storage.get_block_body(number).await? {
        return Ok(Some(body));
    }
    ensure_history_retained(storage, number).await?;
    // Backfill may have stored the body and lowered the earliest block since the first read.
    Ok(storage.get_block_body(number).await?)
}

/// The blob base fee of `header`'s block, as its receipts report it.
fn receipt_blob_base_fee(header: &BlockHeader, storage: &Store) -> Result<u64, RpcErr> {
    let config = storage.get_chain_config();
    let blob_base_fee = calculate_base_fee_per_blob_gas(
        header.excess_blob_gas.unwrap_or_default(),
        config
            .get_fork_blob_schedule(header.timestamp)
            .map(|schedule| schedule.base_fee_update_fraction)
            .unwrap_or_default(),
    );
    blob_base_fee
        .try_into()
        .map_err(|_| RpcErr::Internal("blob_base_fee does not fit in u64".to_owned()))
}

/// Fetches the RPC receipts of every transaction in a block (for `eth_getBlockReceipts`).
pub async fn get_all_block_rpc_receipts(
    header: BlockHeader,
    body: BlockBody,
    storage: &Store,
) -> Result<Vec<RpcReceipt>, RpcErr> {
    let mut receipts = Vec::new();
    // Check if this is the genesis block
    if header.parent_hash.is_zero() {
        return Ok(receipts);
    }
    let blob_base_fee_u64 = receipt_blob_base_fee(&header, storage)?;
    let base_fee_per_gas = header.base_fee_per_gas;
    // Fetch receipt info from block
    let block_number = header.number;
    let block_hash = header.hash();
    let block_timestamp = header.timestamp;
    let block_info = RpcReceiptBlockInfo::from_block_header(header);
    let fetch_count = body.transactions.len();
    let all_receipts = storage
        .get_receipts_for_block_from_index(&block_hash, 0, Some(fetch_count))
        .await?;
    // Return 500 on receipt count mismatch — this indicates data corruption
    // (missing receipts for a block that exists).
    if all_receipts.len() != fetch_count {
        ensure_receipts_retained(storage, block_number).await?;
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
            block_timestamp,
        );
        last_cumulative_gas_used += gas_used;
        current_log_index += receipt.logs.len() as u64;
        receipts.push(receipt);
    }
    Ok(receipts)
}

/// Fetches the RPC receipt of the transaction at `index` in a block (for
/// `eth_getTransactionReceipt`), or `None` if the block has no such transaction.
///
/// The receipt's gas used and first log index depend on the receipts before it,
/// so receipts `0..=index` are read, but only the target transaction's sender and
/// hash are computed: the transactions before it contribute nothing else to the
/// result, and recovering each of their senders made the cost of the call grow
/// with the transaction's position in the block.
pub async fn get_block_rpc_receipt(
    header: BlockHeader,
    mut body: BlockBody,
    storage: &Store,
    index: u64,
) -> Result<Option<RpcReceipt>, RpcErr> {
    // Check if this is the genesis block
    if header.parent_hash.is_zero() {
        return Ok(None);
    }
    let blob_base_fee_u64 = receipt_blob_base_fee(&header, storage)?;
    let base_fee_per_gas = header.base_fee_per_gas;
    let block_number = header.number;
    let block_hash = header.hash();
    let block_timestamp = header.timestamp;
    let block_info = RpcReceiptBlockInfo::from_block_header(header);
    let fetch_count = (index + 1) as usize;
    let mut receipts = storage
        .get_receipts_for_block_from_index(&block_hash, 0, Some(fetch_count))
        .await?;
    // Return 500 on receipt count mismatch — this indicates data corruption
    // (missing receipts for a block that exists).
    if receipts.len() != fetch_count {
        ensure_receipts_retained(storage, block_number).await?;
        return Err(RpcErr::Internal(format!(
            "Expected {} receipts, got {}",
            fetch_count,
            receipts.len()
        )));
    }
    let tx_position = index as usize;
    if tx_position >= body.transactions.len() {
        return Ok(None);
    }
    let tx = body.transactions.swap_remove(tx_position);
    let Some(receipt) = receipts.pop() else {
        return Ok(None);
    };
    let previous_cumulative_gas_used = receipts
        .last()
        .map_or(0, |previous| previous.cumulative_gas_used);
    let first_log_index = receipts
        .iter()
        .map(|previous| previous.logs.len() as u64)
        .sum();
    let gas_used = receipt.cumulative_gas_used - previous_cumulative_gas_used;
    let tx_info = RpcReceiptTxInfo::from_transaction(
        tx,
        index,
        gas_used,
        blob_base_fee_u64,
        base_fee_per_gas,
    )?;
    Ok(Some(RpcReceipt::new(
        receipt,
        tx_info,
        block_info,
        first_log_index,
        block_timestamp,
    )))
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
    let receipts = storage.get_receipts_for_block(&block_hash).await?;
    // `get_receipts_for_block` returns a bare Vec, so a block whose receipts are
    // absent is indistinguishable from a block that genuinely has none. Returning
    // the empty list would be a wrong answer rather than a reported failure, so
    // check it against the block's own transaction count. This mirrors the
    // mismatch check the by-index receipt path already performs.
    let expected = match storage.get_block_body_by_hash(block_hash).await? {
        Some(body) => body.transactions.len(),
        // No body means the block's history is not retained; without it there is
        // nothing to validate the receipt count against.
        None => {
            ensure_history_retained(storage, header.number).await?;
            return Err(RpcErr::Internal(format!(
                "Body unavailable for block {block_hash:#x}, cannot serve its receipts"
            )));
        }
    };
    if receipts.len() != expected {
        ensure_receipts_retained(storage, header.number).await?;
        return Err(RpcErr::Internal(format!(
            "Expected {expected} receipts for block {block_hash:#x}, got {}",
            receipts.len()
        )));
    }
    Ok(receipts)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::test_utils::setup_store;
    use ethrex_common::{
        Address, Bytes, H256, U256,
        types::{EIP1559Transaction, Log, Transaction, TxKind, TxType},
    };
    use ethrex_crypto::{Crypto, NativeCrypto};

    fn signed_tx(secret: &secp256k1::SecretKey, nonce: u64) -> Transaction {
        let mut tx = EIP1559Transaction {
            chain_id: 1,
            nonce,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 2_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(Address::repeat_byte(0xaa)),
            value: U256::one(),
            ..Default::default()
        };
        let (payload, _) = Transaction::EIP1559Transaction(tx.clone())
            .signing_payload()
            .unwrap()
            .unwrap();
        let msg = secp256k1::Message::from_digest(NativeCrypto.keccak256(&payload));
        let (recovery_id, signature) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(&msg, secret)
            .serialize_compact();
        tx.signature_r = U256::from_big_endian(&signature[..32]);
        tx.signature_s = U256::from_big_endian(&signature[32..]);
        tx.signature_y_parity = U256::from(u8::from(i32::from(recovery_id) != 0));
        Transaction::EIP1559Transaction(tx)
    }

    /// Stores a block of `tx_count` signed transactions whose receipts carry
    /// `index % 3` logs each and differing gas, and returns its header and body.
    async fn store_block(store: &Store, tx_count: u64) -> (BlockHeader, BlockBody) {
        let secret = secp256k1::SecretKey::from_byte_array(&[0x11; 32]).unwrap();
        let transactions = (0..tx_count).map(|i| signed_tx(&secret, i)).collect();
        let mut cumulative_gas_used = 0;
        let receipts = (0..tx_count)
            .map(|i| {
                let logs = (0..i % 3)
                    .map(|j| Log {
                        address: Address::repeat_byte(j as u8),
                        topics: vec![H256::repeat_byte(i as u8)],
                        data: Bytes::from(vec![j as u8; 32]),
                    })
                    .collect();
                cumulative_gas_used += 21_000 + i * 100;
                Receipt::new(TxType::EIP1559, i % 4 != 0, cumulative_gas_used, logs)
            })
            .collect();
        let header = BlockHeader {
            parent_hash: H256::repeat_byte(1),
            number: 1,
            base_fee_per_gas: Some(1_000_000_000),
            excess_blob_gas: Some(0),
            blob_gas_used: Some(0),
            timestamp: 1000,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let body = BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: None,
        };
        let block = Block::new(header.clone(), body.clone());
        let hash = block.hash();
        store.add_block(block).await.unwrap();
        store.add_receipts(hash, receipts).await.unwrap();
        (header, body)
    }

    #[tokio::test]
    async fn single_receipt_matches_the_block_receipts() {
        let store = setup_store().await;
        let tx_count = 10;
        let (header, body) = store_block(&store, tx_count).await;
        let all = get_all_block_rpc_receipts(header.clone(), body.clone(), &store)
            .await
            .unwrap();
        assert_eq!(all.len(), tx_count as usize);
        for (index, expected) in all.iter().enumerate() {
            let single = get_block_rpc_receipt(header.clone(), body.clone(), &store, index as u64)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::to_value(&single).unwrap(),
                serde_json::to_value(expected).unwrap(),
                "receipt {index}"
            );
        }
    }

    #[tokio::test]
    async fn single_receipt_beyond_the_block_is_an_error() {
        let store = setup_store().await;
        let (header, body) = store_block(&store, 3).await;
        assert!(
            get_block_rpc_receipt(header, body, &store, 3)
                .await
                .is_err()
        );
    }

    async fn dispatch(
        context: &RpcApiContext,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Value, RpcErr> {
        let request: crate::utils::RpcRequest = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .unwrap();
        crate::rpc::map_http_requests(&request, context.clone()).await
    }

    fn block_history_reads(number: &str, hash: &str) -> Vec<(&'static str, serde_json::Value)> {
        use serde_json::json;
        vec![
            ("eth_getBlockByNumber", json!([number, false])),
            ("eth_getBlockByNumber", json!([number, true])),
            ("eth_getBlockByHash", json!([hash, true])),
            ("eth_getBlockTransactionCountByNumber", json!([number])),
            ("eth_getBlockTransactionCountByHash", json!([hash])),
            ("eth_getUncleCountByBlockNumber", json!([number])),
            ("eth_getUncleCountByBlockHash", json!([hash])),
            (
                "eth_getTransactionByBlockNumberAndIndex",
                json!([number, "0x0"]),
            ),
            (
                "eth_getTransactionByBlockHashAndIndex",
                json!([hash, "0x0"]),
            ),
            (
                "eth_getRawTransactionByBlockNumberAndIndex",
                json!([number, "0x0"]),
            ),
            (
                "eth_getRawTransactionByBlockHashAndIndex",
                json!([hash, "0x0"]),
            ),
            ("eth_getBlockReceipts", json!([number])),
            ("eth_getBlockReceipts", json!([hash])),
            ("debug_getRawBlock", json!([number])),
            ("debug_getRawReceipts", json!([number])),
            (
                "eth_getLogs",
                json!([{"fromBlock": number, "toBlock": number}]),
            ),
            ("eth_getLogs", json!([{"blockHash": hash}])),
            ("eth_feeHistory", json!(["0x1", number, [50.0]])),
        ]
    }

    /// Block 1 is header-only (or body without receipts if `body_of_block_one`), block 2 full.
    async fn store_with_history_from_block_two(
        store: &Store,
        body_of_block_one: bool,
    ) -> (BlockHash, Block, Receipt) {
        let secret = secp256k1::SecretKey::from_byte_array(&[0x11; 32]).unwrap();
        let genesis = store.get_block_header(0).unwrap().unwrap();
        let block_one = BlockHeader {
            parent_hash: genesis.hash(),
            number: 1,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let block_one_hash = block_one.hash();
        if body_of_block_one {
            let body = BlockBody {
                transactions: vec![signed_tx(&secret, 0)],
                ommers: vec![],
                withdrawals: None,
            };
            store.add_block(Block::new(block_one, body)).await.unwrap();
        } else {
            store.add_block_headers(vec![block_one]).await.unwrap();
        }

        let log = Log {
            address: Address::repeat_byte(0xaa),
            topics: vec![H256::repeat_byte(1)],
            data: Bytes::new(),
        };
        let block_two = BlockHeader {
            parent_hash: block_one_hash,
            number: 2,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(1_000_000_000),
            ..Default::default()
        };
        let body = BlockBody {
            transactions: vec![signed_tx(&secret, 1)],
            ommers: vec![],
            withdrawals: None,
        };
        let block_two = Block::new(block_two, body);
        let block_two_hash = block_two.hash();
        let receipt = Receipt::new(TxType::EIP1559, true, 21_000, vec![log]);
        store.add_block(block_two.clone()).await.unwrap();
        store
            .add_receipts(block_two_hash, vec![receipt.clone()])
            .await
            .unwrap();
        store
            .forkchoice_update(vec![(1, block_one_hash)], 2, block_two_hash, None, None)
            .await
            .unwrap();
        store.update_earliest_block_number(2).await.unwrap();
        (block_one_hash, block_two, receipt)
    }

    #[tokio::test]
    async fn reads_of_a_block_without_its_body_are_pruned_history() {
        use serde_json::json;
        let store = setup_store().await;
        let (block_one, block_two, receipt) =
            store_with_history_from_block_two(&store, false).await;
        let context = crate::test_utils::context_without_p2p(store);

        for (method, params) in block_history_reads("0x1", &format!("{block_one:#x}")) {
            let result = dispatch(&context, method, params.clone()).await;
            assert!(
                matches!(result, Err(RpcErr::PrunedHistoryUnavailable(_))),
                "{method} {params} gave {result:?}"
            );
        }

        let other_address = format!("{:#x}", Address::repeat_byte(0xbb));
        for bloom_excluded in [
            json!([{"fromBlock": "0x1", "toBlock": "0x1", "address": other_address}]),
            json!([{"fromBlock": "0x1", "toBlock": "0x2", "address": other_address}]),
            json!([{"blockHash": format!("{block_one:#x}"), "address": other_address}]),
        ] {
            let result = dispatch(&context, "eth_getLogs", bloom_excluded.clone()).await;
            assert!(
                matches!(result, Err(RpcErr::PrunedHistoryUnavailable(_))),
                "{bloom_excluded} gave {result:?}"
            );
        }

        let fee_history = dispatch(&context, "eth_feeHistory", json!(["0x2", "0x2", []]))
            .await
            .unwrap();
        assert_eq!(fee_history["oldestBlock"], "0x1");
        assert_eq!(fee_history["gasUsedRatio"], json!([0.0, 0.0]));
        let with_rewards = dispatch(&context, "eth_feeHistory", json!(["0x2", "0x2", [50.0]]))
            .await
            .unwrap();
        assert_eq!(with_rewards["oldestBlock"], "0x2");
        assert_eq!(with_rewards["gasUsedRatio"], json!([0.0]));
        assert_eq!(with_rewards["reward"], json!([["0x1"]]));
        let below_earliest = dispatch(&context, "eth_feeHistory", json!(["0x1", "0x1", []]))
            .await
            .unwrap();
        assert_eq!(below_earliest["oldestBlock"], "0x1");
        assert_eq!(below_earliest["gasUsedRatio"], json!([0.0]));
        assert_eq!(below_earliest["baseFeePerGas"].as_array().unwrap().len(), 2);

        let hash = json!(format!("{:#x}", block_two.hash()));
        let tx = &block_two.body.transactions[0];
        let tx_hash = json!(format!("{:#x}", tx.hash(&NativeCrypto)));
        let log_address = json!(format!("{:#x}", Address::repeat_byte(0xaa)));
        let raw_tx = json!(format!("0x{}", hex::encode(tx.encode_canonical_to_vec())));
        let raw_block = json!(format!("0x{}", hex::encode(block_two.encode_to_vec())));
        let raw_receipts = json!([format!(
            "0x{}",
            hex::encode(receipt.encode_inner_with_bloom(&NativeCrypto))
        )]);
        for (method, params) in block_history_reads("0x2", &format!("{:#x}", block_two.hash())) {
            let result = dispatch(&context, method, params.clone()).await;
            let value = result.unwrap_or_else(|err| panic!("{method} {params} gave {err:?}"));
            let answered = match method {
                "eth_getBlockByNumber" | "eth_getBlockByHash" => {
                    let first_tx = &value["transactions"][0];
                    value["hash"] == hash && (*first_tx == tx_hash || first_tx["hash"] == tx_hash)
                }
                "eth_getBlockTransactionCountByNumber" | "eth_getBlockTransactionCountByHash" => {
                    value == "0x1"
                }
                "eth_getUncleCountByBlockNumber" | "eth_getUncleCountByBlockHash" => value == "0x0",
                "eth_getTransactionByBlockNumberAndIndex"
                | "eth_getTransactionByBlockHashAndIndex" => {
                    value["hash"] == tx_hash && value["blockHash"] == hash
                }
                "eth_getRawTransactionByBlockNumberAndIndex"
                | "eth_getRawTransactionByBlockHashAndIndex" => value == raw_tx,
                "eth_getBlockReceipts" => {
                    value[0]["transactionHash"] == tx_hash
                        && value[0]["logs"][0]["address"] == log_address
                }
                "debug_getRawBlock" => value == raw_block,
                "debug_getRawReceipts" => value == raw_receipts,
                "eth_getLogs" => {
                    value[0]["transactionHash"] == tx_hash
                        && value[0]["blockHash"] == hash
                        && value[0]["address"] == log_address
                }
                "eth_feeHistory" => {
                    value["oldestBlock"] == "0x2" && value["reward"] == json!([["0x1"]])
                }
                _ => unreachable!("{method}"),
            };
            assert!(answered, "{method} {params} gave {value}");
        }
    }

    #[tokio::test]
    async fn reads_of_an_unknown_block_are_unchanged() {
        let store = setup_store().await;
        store_with_history_from_block_two(&store, false).await;
        let context = crate::test_utils::context_without_p2p(store);
        let unknown_hash = format!("{:#x}", H256::repeat_byte(0xbd));

        for (method, params) in block_history_reads("0x10", &unknown_hash) {
            let result = dispatch(&context, method, params.clone()).await;
            match method {
                "eth_getLogs" => assert!(
                    matches!(
                        result,
                        Err(RpcErr::InvalidParams(_) | RpcErr::ServerError(_))
                    ),
                    "{method} {params} gave {result:?}"
                ),
                "eth_feeHistory" => assert!(result.is_ok(), "{method} {params} gave {result:?}"),
                _ => assert_eq!(result.unwrap(), Value::Null, "{method} {params}"),
            }
        }
    }

    /// Earliest 1 is the snap pivot: its body is stored, its receipts are not.
    #[tokio::test]
    async fn missing_receipts_are_pruned_history_up_to_the_earliest_block() {
        let store = setup_store().await;
        let (block_one, _, _) = store_with_history_from_block_two(&store, true).await;
        let tx_hash =
            store.get_block_body(1).await.unwrap().unwrap().transactions[0].hash(&NativeCrypto);
        let context = crate::test_utils::context_without_p2p(store.clone());
        let receipt_reads = [
            (
                "eth_getTransactionReceipt",
                serde_json::json!([format!("{tx_hash:#x}")]),
            ),
            ("eth_getBlockReceipts", serde_json::json!(["0x1"])),
            ("debug_getRawReceipts", serde_json::json!(["0x1"])),
            (
                "eth_getLogs",
                serde_json::json!([{"fromBlock": "0x1", "toBlock": "0x1"}]),
            ),
            (
                "eth_getLogs",
                serde_json::json!([{"blockHash": format!("{block_one:#x}")}]),
            ),
        ];

        for earliest in [2, 1] {
            store.update_earliest_block_number(earliest).await.unwrap();
            for (method, params) in receipt_reads.clone() {
                let result = dispatch(&context, method, params.clone()).await;
                assert!(
                    matches!(result, Err(RpcErr::PrunedHistoryUnavailable(_))),
                    "earliest {earliest}: {method} {params} gave {result:?}"
                );
            }
        }
        let Err(RpcErr::PrunedHistoryUnavailable(message)) =
            dispatch(&context, "eth_getBlockReceipts", serde_json::json!(["0x1"])).await
        else {
            unreachable!()
        };
        assert!(
            message.ends_with("receipts are available from block 2"),
            "{message}"
        );

        store.update_earliest_block_number(0).await.unwrap();
        for (method, params) in receipt_reads {
            let result = dispatch(&context, method, params.clone()).await;
            assert!(
                matches!(result, Err(RpcErr::Internal(_))),
                "{method} {params} gave {result:?}"
            );
        }
    }
}
