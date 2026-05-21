//! Conversions between SSZ wire types and ethrex internal types.

use bytes::Bytes;
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::requests::EncodedRequests;
use ethrex_common::types::{
    BlobsBundle, Block, BlockBody, BlockHeader, Transaction, Withdrawal, compute_transactions_root,
    compute_withdrawals_root,
};
use ethrex_common::{Address, Bloom, H256};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use libssz_types::SszList;

use crate::engine_rest::error::ConversionError;
use crate::engine_rest::types::blobs::{BlobsBundleV1, BlobsBundleV2};
use crate::engine_rest::types::common::{
    Bytes32, MAX_BLOB_COMMITMENTS_PER_BLOCK, MAX_BYTES_PER_TRANSACTION, MAX_EXTRA_DATA_BYTES,
    PayloadStatusCode, PayloadStatusV1 as SszPayloadStatus, ssz_none, ssz_some, u64_to_uint256_le,
    uint256_le_to_u64,
};
use crate::engine_rest::types::execution_payload::{
    ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, ExecutionPayloadV4, Transactions,
    Withdrawals,
};
use crate::engine_rest::types::new_payload::ExecutionRequests;
use crate::engine_rest::types::withdrawal::WithdrawalV1;
use crate::types::payload::{
    EncodedTransaction, ExecutionPayload as JsonExecutionPayload,
    PayloadStatus as JsonPayloadStatus, PayloadValidationStatus,
};

pub fn encoded_transactions_to_ssz(
    txs: &[EncodedTransaction],
) -> Result<Transactions, ConversionError> {
    let inner: Result<Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>>, _> =
        txs.iter().map(|tx| tx.0.to_vec().try_into()).collect();
    inner
        .map_err(|_| ConversionError::internal("transaction exceeds MAX_BYTES_PER_TRANSACTION"))?
        .try_into()
        .map_err(|_| ConversionError::internal("tx count exceeds MAX_TRANSACTIONS_PER_PAYLOAD"))
}

pub fn ssz_withdrawals_to_vec(ws: &Withdrawals) -> Vec<Withdrawal> {
    ws.iter()
        .map(|w| Withdrawal {
            index: w.index,
            validator_index: w.validator_index,
            address: Address::from_slice(&w.address),
            amount: w.amount,
        })
        .collect()
}

pub fn vec_withdrawals_to_ssz(ws: &[Withdrawal]) -> Result<Withdrawals, ConversionError> {
    let v: Vec<WithdrawalV1> = ws
        .iter()
        .map(|w| WithdrawalV1 {
            index: w.index,
            validator_index: w.validator_index,
            address: w.address.0,
            amount: w.amount,
        })
        .collect();
    v.try_into().map_err(|_| {
        ConversionError::internal("withdrawal count exceeds MAX_WITHDRAWALS_PER_PAYLOAD")
    })
}

fn ssz_extra_data_to_bytes(e: &SszList<u8, MAX_EXTRA_DATA_BYTES>) -> Bytes {
    Bytes::copy_from_slice(e)
}

fn bytes_to_ssz_extra_data(
    b: &Bytes,
) -> Result<SszList<u8, MAX_EXTRA_DATA_BYTES>, ConversionError> {
    b.to_vec()
        .try_into()
        .map_err(|_| ConversionError::internal("extra_data exceeds MAX_EXTRA_DATA_BYTES"))
}

fn empty_withdrawals() -> Result<Withdrawals, ConversionError> {
    Vec::<WithdrawalV1>::new()
        .try_into()
        .map_err(|_| ConversionError::internal("empty withdrawals list overflow"))
}

fn decode_bal(bytes: &[u8]) -> Result<BlockAccessList, ConversionError> {
    BlockAccessList::decode(bytes)
        .map_err(|e: RLPDecodeError| ConversionError::bad_request(format!("invalid BAL RLP: {e}")))
}

pub fn json_to_execution_payload_v1(
    p: &JsonExecutionPayload,
) -> Result<ExecutionPayloadV1, ConversionError> {
    Ok(ExecutionPayloadV1 {
        parent_hash: p.parent_hash.0,
        fee_recipient: p.fee_recipient.0,
        state_root: p.state_root.0,
        receipts_root: p.receipts_root.0,
        logs_bloom: p.logs_bloom.0,
        prev_randao: p.prev_randao.0,
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: bytes_to_ssz_extra_data(&p.extra_data)?,
        base_fee_per_gas: u64_to_uint256_le(p.base_fee_per_gas),
        block_hash: p.block_hash.0,
        transactions: encoded_transactions_to_ssz(&p.transactions)?,
    })
}

pub fn json_to_execution_payload_v2(
    p: &JsonExecutionPayload,
) -> Result<ExecutionPayloadV2, ConversionError> {
    let withdrawals = match &p.withdrawals {
        Some(ws) => vec_withdrawals_to_ssz(ws)?,
        None => empty_withdrawals()?,
    };
    Ok(ExecutionPayloadV2 {
        parent_hash: p.parent_hash.0,
        fee_recipient: p.fee_recipient.0,
        state_root: p.state_root.0,
        receipts_root: p.receipts_root.0,
        logs_bloom: p.logs_bloom.0,
        prev_randao: p.prev_randao.0,
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: bytes_to_ssz_extra_data(&p.extra_data)?,
        base_fee_per_gas: u64_to_uint256_le(p.base_fee_per_gas),
        block_hash: p.block_hash.0,
        transactions: encoded_transactions_to_ssz(&p.transactions)?,
        withdrawals,
    })
}

pub fn json_to_execution_payload_v3(
    p: &JsonExecutionPayload,
) -> Result<ExecutionPayloadV3, ConversionError> {
    let withdrawals = match &p.withdrawals {
        Some(ws) => vec_withdrawals_to_ssz(ws)?,
        None => empty_withdrawals()?,
    };
    Ok(ExecutionPayloadV3 {
        parent_hash: p.parent_hash.0,
        fee_recipient: p.fee_recipient.0,
        state_root: p.state_root.0,
        receipts_root: p.receipts_root.0,
        logs_bloom: p.logs_bloom.0,
        prev_randao: p.prev_randao.0,
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: bytes_to_ssz_extra_data(&p.extra_data)?,
        base_fee_per_gas: u64_to_uint256_le(p.base_fee_per_gas),
        block_hash: p.block_hash.0,
        transactions: encoded_transactions_to_ssz(&p.transactions)?,
        withdrawals,
        blob_gas_used: p.blob_gas_used.unwrap_or(0),
        excess_blob_gas: p.excess_blob_gas.unwrap_or(0),
    })
}

pub fn json_to_execution_payload_v4(
    p: &JsonExecutionPayload,
) -> Result<ExecutionPayloadV4, ConversionError> {
    let withdrawals = match &p.withdrawals {
        Some(ws) => vec_withdrawals_to_ssz(ws)?,
        None => empty_withdrawals()?,
    };
    let bal_bytes: Vec<u8> = match &p.block_access_list {
        Some(b) => {
            let mut buf = Vec::new();
            b.encode(&mut buf);
            buf
        }
        None => Vec::new(),
    };
    let bal_ssz = bal_bytes
        .try_into()
        .map_err(|_| ConversionError::internal("BAL RLP exceeds MAX_BYTES_PER_TRANSACTION"))?;
    Ok(ExecutionPayloadV4 {
        parent_hash: p.parent_hash.0,
        fee_recipient: p.fee_recipient.0,
        state_root: p.state_root.0,
        receipts_root: p.receipts_root.0,
        logs_bloom: p.logs_bloom.0,
        prev_randao: p.prev_randao.0,
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: bytes_to_ssz_extra_data(&p.extra_data)?,
        base_fee_per_gas: u64_to_uint256_le(p.base_fee_per_gas),
        block_hash: p.block_hash.0,
        transactions: encoded_transactions_to_ssz(&p.transactions)?,
        withdrawals,
        blob_gas_used: p.blob_gas_used.unwrap_or(0),
        excess_blob_gas: p.excess_blob_gas.unwrap_or(0),
        block_access_list: bal_ssz,
        slot_number: p.slot_number.unwrap_or(0),
    })
}

pub fn json_payload_status_to_ssz(
    s: &JsonPayloadStatus,
) -> Result<SszPayloadStatus, ConversionError> {
    let code: u8 = match s.status {
        PayloadValidationStatus::Valid => PayloadStatusCode::Valid as u8,
        PayloadValidationStatus::Invalid => PayloadStatusCode::Invalid as u8,
        PayloadValidationStatus::Syncing => PayloadStatusCode::Syncing as u8,
        PayloadValidationStatus::Accepted => PayloadStatusCode::Accepted as u8,
    };
    let latest_valid_hash = match s.latest_valid_hash {
        Some(h) => ssz_some(h.0),
        None => ssz_none(),
    };
    let validation_error = s
        .validation_error
        .as_deref()
        .unwrap_or("")
        .as_bytes()
        .to_vec()
        .try_into()
        .map_err(|_| {
            ConversionError::internal("validation_error exceeds MAX_ERROR_MESSAGE_LENGTH")
        })?;
    Ok(SszPayloadStatus {
        status: code,
        latest_valid_hash,
        validation_error,
    })
}

pub fn blobs_bundle_to_ssz_v1(bundle: BlobsBundle) -> Result<BlobsBundleV1, ConversionError> {
    Ok(BlobsBundleV1 {
        commitments: bundle
            .commitments
            .try_into()
            .map_err(|_| ConversionError::internal("commitments overflow"))?,
        proofs: bundle
            .proofs
            .try_into()
            .map_err(|_| ConversionError::internal("proofs overflow"))?,
        blobs: bundle
            .blobs
            .try_into()
            .map_err(|_| ConversionError::internal("blobs overflow"))?,
    })
}

pub fn blobs_bundle_to_ssz_v2(bundle: BlobsBundle) -> Result<BlobsBundleV2, ConversionError> {
    Ok(BlobsBundleV2 {
        commitments: bundle
            .commitments
            .try_into()
            .map_err(|_| ConversionError::internal("commitments overflow"))?,
        proofs: bundle
            .proofs
            .try_into()
            .map_err(|_| ConversionError::internal("proofs overflow"))?,
        blobs: bundle
            .blobs
            .try_into()
            .map_err(|_| ConversionError::internal("blobs overflow"))?,
    })
}

pub fn encoded_requests_to_ssz(
    reqs: &[EncodedRequests],
) -> Result<ExecutionRequests, ConversionError> {
    let inner: Result<Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>>, _> = reqs
        .iter()
        .filter(|r| !r.0.is_empty())
        .map(|r| r.0.to_vec().try_into())
        .collect();
    inner
        .map_err(|_| {
            ConversionError::internal("execution request exceeds MAX_BYTES_PER_TRANSACTION")
        })?
        .try_into()
        .map_err(|_| ConversionError::internal("execution_requests overflow"))
}

pub fn ssz_to_encoded_requests(reqs: &ExecutionRequests) -> Vec<EncodedRequests> {
    reqs.iter()
        .map(|r| EncodedRequests(Bytes::copy_from_slice(r)))
        .collect()
}

pub fn ssz_blob_hashes_to_vec(
    hashes: &SszList<Bytes32, MAX_BLOB_COMMITMENTS_PER_BLOCK>,
) -> Vec<H256> {
    hashes.iter().map(H256::from).collect()
}

// Direct SSZ → Block: decode each tx slice into a `Transaction` without
// going through `EncodedTransaction(Bytes)` or `JsonExecutionPayload`.

fn decode_transactions(txs: &Transactions) -> Result<Vec<Transaction>, ConversionError> {
    txs.iter()
        .map(|raw| Transaction::decode_canonical(raw))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ConversionError::bad_request(format!("invalid transaction: {e}")))
}

pub fn ssz_payload_v1_to_block(
    p: ExecutionPayloadV1,
    parent_beacon_block_root: Option<H256>,
    requests_hash: Option<H256>,
    block_access_list_hash: Option<H256>,
) -> Result<Block, ConversionError> {
    let base_fee = uint256_le_to_u64(&p.base_fee_per_gas)
        .ok_or_else(|| ConversionError::bad_request("base_fee_per_gas exceeds u64"))?;
    let transactions = decode_transactions(&p.transactions)?;
    let body = BlockBody {
        transactions,
        ommers: vec![],
        withdrawals: None,
    };
    let header = BlockHeader {
        parent_hash: H256::from(p.parent_hash),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::from(p.fee_recipient),
        state_root: H256::from(p.state_root),
        transactions_root: compute_transactions_root(&body.transactions, &NativeCrypto),
        receipts_root: H256::from(p.receipts_root),
        logs_bloom: Bloom::from_slice(&p.logs_bloom),
        difficulty: 0.into(),
        number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: ssz_extra_data_to_bytes(&p.extra_data),
        prev_randao: H256::from(p.prev_randao),
        nonce: 0,
        base_fee_per_gas: Some(base_fee),
        withdrawals_root: None,
        blob_gas_used: None,
        excess_blob_gas: None,
        parent_beacon_block_root,
        requests_hash,
        slot_number: None,
        block_access_list_hash,
        ..Default::default()
    };
    Ok(Block::new(header, body))
}

pub fn ssz_payload_v2_to_block(
    p: ExecutionPayloadV2,
    parent_beacon_block_root: Option<H256>,
    requests_hash: Option<H256>,
    block_access_list_hash: Option<H256>,
) -> Result<Block, ConversionError> {
    let base_fee = uint256_le_to_u64(&p.base_fee_per_gas)
        .ok_or_else(|| ConversionError::bad_request("base_fee_per_gas exceeds u64"))?;
    let transactions = decode_transactions(&p.transactions)?;
    let withdrawals = Some(ssz_withdrawals_to_vec(&p.withdrawals));
    let withdrawals_root = withdrawals
        .as_ref()
        .map(|w| compute_withdrawals_root(w, &NativeCrypto));
    let body = BlockBody {
        transactions,
        ommers: vec![],
        withdrawals,
    };
    let header = BlockHeader {
        parent_hash: H256::from(p.parent_hash),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::from(p.fee_recipient),
        state_root: H256::from(p.state_root),
        transactions_root: compute_transactions_root(&body.transactions, &NativeCrypto),
        receipts_root: H256::from(p.receipts_root),
        logs_bloom: Bloom::from_slice(&p.logs_bloom),
        difficulty: 0.into(),
        number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: ssz_extra_data_to_bytes(&p.extra_data),
        prev_randao: H256::from(p.prev_randao),
        nonce: 0,
        base_fee_per_gas: Some(base_fee),
        withdrawals_root,
        blob_gas_used: None,
        excess_blob_gas: None,
        parent_beacon_block_root,
        requests_hash,
        slot_number: None,
        block_access_list_hash,
        ..Default::default()
    };
    Ok(Block::new(header, body))
}

pub fn ssz_payload_v3_to_block(
    p: ExecutionPayloadV3,
    parent_beacon_block_root: Option<H256>,
    requests_hash: Option<H256>,
    block_access_list_hash: Option<H256>,
) -> Result<Block, ConversionError> {
    let base_fee = uint256_le_to_u64(&p.base_fee_per_gas)
        .ok_or_else(|| ConversionError::bad_request("base_fee_per_gas exceeds u64"))?;
    let transactions = decode_transactions(&p.transactions)?;
    let withdrawals = Some(ssz_withdrawals_to_vec(&p.withdrawals));
    let withdrawals_root = withdrawals
        .as_ref()
        .map(|w| compute_withdrawals_root(w, &NativeCrypto));
    let body = BlockBody {
        transactions,
        ommers: vec![],
        withdrawals,
    };
    let header = BlockHeader {
        parent_hash: H256::from(p.parent_hash),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::from(p.fee_recipient),
        state_root: H256::from(p.state_root),
        transactions_root: compute_transactions_root(&body.transactions, &NativeCrypto),
        receipts_root: H256::from(p.receipts_root),
        logs_bloom: Bloom::from_slice(&p.logs_bloom),
        difficulty: 0.into(),
        number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: ssz_extra_data_to_bytes(&p.extra_data),
        prev_randao: H256::from(p.prev_randao),
        nonce: 0,
        base_fee_per_gas: Some(base_fee),
        withdrawals_root,
        blob_gas_used: Some(p.blob_gas_used),
        excess_blob_gas: Some(p.excess_blob_gas),
        parent_beacon_block_root,
        requests_hash,
        slot_number: None,
        block_access_list_hash,
        ..Default::default()
    };
    Ok(Block::new(header, body))
}

/// Returns `(Block, Option<BlockAccessList>)`. The BAL is decoded from the
/// SSZ payload's `block_access_list` bytes and returned separately for the
/// caller to pass to `handle_new_payload_v4`. An empty SSZ BAL maps to `None`.
pub fn ssz_payload_v4_to_block(
    p: ExecutionPayloadV4,
    parent_beacon_block_root: Option<H256>,
    requests_hash: Option<H256>,
    block_access_list_hash: Option<H256>,
) -> Result<(Block, Option<BlockAccessList>), ConversionError> {
    let base_fee = uint256_le_to_u64(&p.base_fee_per_gas)
        .ok_or_else(|| ConversionError::bad_request("base_fee_per_gas exceeds u64"))?;
    let transactions = decode_transactions(&p.transactions)?;
    let withdrawals = Some(ssz_withdrawals_to_vec(&p.withdrawals));
    let withdrawals_root = withdrawals
        .as_ref()
        .map(|w| compute_withdrawals_root(w, &NativeCrypto));
    let bal = if p.block_access_list.is_empty() {
        None
    } else {
        Some(decode_bal(&p.block_access_list)?)
    };
    let body = BlockBody {
        transactions,
        ommers: vec![],
        withdrawals,
    };
    let header = BlockHeader {
        parent_hash: H256::from(p.parent_hash),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::from(p.fee_recipient),
        state_root: H256::from(p.state_root),
        transactions_root: compute_transactions_root(&body.transactions, &NativeCrypto),
        receipts_root: H256::from(p.receipts_root),
        logs_bloom: Bloom::from_slice(&p.logs_bloom),
        difficulty: 0.into(),
        number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: ssz_extra_data_to_bytes(&p.extra_data),
        prev_randao: H256::from(p.prev_randao),
        nonce: 0,
        base_fee_per_gas: Some(base_fee),
        withdrawals_root,
        blob_gas_used: Some(p.blob_gas_used),
        excess_blob_gas: Some(p.excess_blob_gas),
        parent_beacon_block_root,
        requests_hash,
        slot_number: Some(p.slot_number),
        block_access_list_hash,
        ..Default::default()
    };
    Ok((Block::new(header, body), bal))
}
