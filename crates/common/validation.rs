//! Stateless block validation utilities.
//!
//! This module provides pure validation functions that can be used without
//! storage dependencies, making them suitable for use in zkVM guest programs.

use crate::constants::{GAS_PER_BLOB, MAX_RLP_BLOCK_SIZE, POST_OSAKA_GAS_LIMIT_CAP};
use crate::errors::InvalidBlockError;
use crate::types::requests::{EncodedRequests, Requests, compute_requests_hash};
use crate::types::{
    Block, BlockHeader, ChainConfig, EIP4844Transaction, Receipt, compute_receipts_root,
    validate_block_header, validate_cancun_header_fields, validate_prague_header_fields,
    validate_pre_cancun_header_fields,
};
use ethrex_rlp::encode::RLPEncode;

/// Performs pre-execution validation of the block's header values in reference to the parent_header.
/// Verifies that blob gas fields in the header are correct in reference to the block's body.
/// If a block passes this check, execution will still fail with execute_block when a transaction runs out of gas.
///
/// # WARNING
///
/// This doesn't validate that the transactions or withdrawals root of the header matches the body
/// contents, since we assume the caller already did it. And, in any case, that wouldn't invalidate the block header.
///
/// To validate it, use [`ethrex_common::types::validate_block_body`]
pub fn validate_block_pre_execution(
    block: &Block,
    parent_header: &BlockHeader,
    chain_config: &ChainConfig,
    elasticity_multiplier: u64,
) -> Result<(), InvalidBlockError> {
    // Verify initial header validity against parent
    validate_block_header(&block.header, parent_header, elasticity_multiplier)?;

    if chain_config.is_osaka_activated(block.header.timestamp) {
        let block_rlp_size = block.length();
        if block_rlp_size > MAX_RLP_BLOCK_SIZE as usize {
            return Err(InvalidBlockError::MaximumRlpSizeExceeded(
                MAX_RLP_BLOCK_SIZE,
                block_rlp_size as u64,
            ));
        }
    }
    if chain_config.is_prague_activated(block.header.timestamp) {
        validate_prague_header_fields(&block.header, parent_header, chain_config)?;
        verify_blob_gas_usage(block, chain_config)?;
        if chain_config.is_osaka_activated(block.header.timestamp) {
            verify_transaction_max_gas_limit(block)?;
        }
    } else if chain_config.is_cancun_activated(block.header.timestamp) {
        validate_cancun_header_fields(&block.header, parent_header, chain_config)?;
        verify_blob_gas_usage(block, chain_config)?;
    } else {
        validate_pre_cancun_header_fields(&block.header)?;
    }

    Ok(())
}

/// Validates that the block gas used matches the block header.
/// For Amsterdam+ (EIP-7778), block_gas_used is PRE-REFUND and differs from
/// receipt cumulative_gas_used which is POST-REFUND.
pub fn validate_gas_used(
    block_gas_used: u64,
    block_header: &BlockHeader,
) -> Result<(), InvalidBlockError> {
    if block_gas_used != block_header.gas_used {
        return Err(InvalidBlockError::GasUsedMismatch(
            block_gas_used,
            block_header.gas_used,
        ));
    }
    Ok(())
}

/// Validates that the receipts root matches the block header.
pub fn validate_receipts_root(
    block_header: &BlockHeader,
    receipts: &[Receipt],
) -> Result<(), InvalidBlockError> {
    let receipts_root = compute_receipts_root(receipts);

    if receipts_root == block_header.receipts_root {
        Ok(())
    } else {
        Err(InvalidBlockError::ReceiptsRootMismatch)
    }
}

/// Validates that the requests hash matches the block header (Prague+).
pub fn validate_requests_hash(
    header: &BlockHeader,
    chain_config: &ChainConfig,
    requests: &[Requests],
) -> Result<(), InvalidBlockError> {
    if !chain_config.is_prague_activated(header.timestamp) {
        return Ok(());
    }

    let encoded_requests: Vec<EncodedRequests> = requests.iter().map(|r| r.encode()).collect();
    let computed_requests_hash = compute_requests_hash(&encoded_requests);
    let valid = header
        .requests_hash
        .map(|requests_hash| requests_hash == computed_requests_hash)
        .unwrap_or(false);

    if !valid {
        return Err(InvalidBlockError::RequestsHashMismatch);
    }

    Ok(())
}

/// Helper to validate that all indices in an iterator are within bounds.
fn validate_bal_indices(
    indices: impl Iterator<Item = u16>,
    max_valid_index: u16,
) -> Result<(), InvalidBlockError> {
    for index in indices {
        if index > max_valid_index {
            return Err(InvalidBlockError::BlockAccessListIndexOutOfBounds {
                index,
                max: max_valid_index,
            });
        }
    }
    Ok(())
}

/// Validates that the block access list hash matches the block header (Amsterdam+).
/// Also validates that all BlockAccessIndex values are within valid bounds per EIP-7928.
pub fn validate_block_access_list_hash(
    header: &BlockHeader,
    chain_config: &ChainConfig,
    computed_bal: &crate::types::block_access_list::BlockAccessList,
    transaction_count: usize,
) -> Result<(), InvalidBlockError> {
    // BAL validation only applies to Amsterdam+ forks
    if !chain_config.is_amsterdam_activated(header.timestamp) {
        return Ok(());
    }

    // Per EIP-7928: "Invalidate block if access list...contains indices exceeding len(transactions) + 1"
    // Index semantics: 0=pre-exec, 1..n=tx indices, n+1=post-exec (withdrawals)
    #[allow(clippy::cast_possible_truncation)]
    let max_valid_index = transaction_count as u16 + 1;

    // Validate all indices in the BAL
    for account in computed_bal.accounts() {
        // Check storage_changes indices
        validate_bal_indices(
            account
                .storage_changes
                .iter()
                .flat_map(|slot| slot.slot_changes.iter().map(|c| c.block_access_index)),
            max_valid_index,
        )?;

        // Check balance_changes indices
        validate_bal_indices(
            account.balance_changes.iter().map(|c| c.block_access_index),
            max_valid_index,
        )?;

        // Check nonce_changes indices
        validate_bal_indices(
            account.nonce_changes.iter().map(|c| c.block_access_index),
            max_valid_index,
        )?;

        // Check code_changes indices
        validate_bal_indices(
            account.code_changes.iter().map(|c| c.block_access_index),
            max_valid_index,
        )?;
    }

    let computed_hash = computed_bal.compute_hash();
    let valid = header
        .block_access_list_hash
        .map(|expected_hash| expected_hash == computed_hash)
        .unwrap_or(false);

    if !valid {
        return Err(InvalidBlockError::BlockAccessListHashMismatch);
    }

    Ok(())
}

/// Perform validations over the block's blob gas usage.
/// Must be called only if the block has cancun activated.
fn verify_blob_gas_usage(block: &Block, config: &ChainConfig) -> Result<(), InvalidBlockError> {
    let mut blob_gas_used = 0_u32;
    let mut blobs_in_block = 0_u32;
    let max_blob_number_per_block = config
        .get_fork_blob_schedule(block.header.timestamp)
        .map(|schedule| schedule.max)
        .ok_or(InvalidBlockError::InvalidBlockFork)?;
    let max_blob_gas_per_block = max_blob_number_per_block * GAS_PER_BLOB;

    for transaction in block.body.transactions.iter() {
        match transaction {
            crate::types::Transaction::EIP4844Transaction(tx) => {
                blob_gas_used += get_total_blob_gas(tx);
                blobs_in_block += tx.blob_versioned_hashes.len() as u32;
            }
            crate::types::Transaction::FrameTransaction(tx) => {
                blob_gas_used += GAS_PER_BLOB * tx.blob_versioned_hashes.len() as u32;
                blobs_in_block += tx.blob_versioned_hashes.len() as u32;
            }
            _ => {}
        }
    }
    if blob_gas_used > max_blob_gas_per_block {
        return Err(InvalidBlockError::ExceededMaxBlobGasPerBlock);
    }
    if blobs_in_block > max_blob_number_per_block {
        return Err(InvalidBlockError::ExceededMaxBlobNumberPerBlock);
    }
    if block
        .header
        .blob_gas_used
        .is_some_and(|header_blob_gas_used| header_blob_gas_used != blob_gas_used as u64)
    {
        return Err(InvalidBlockError::BlobGasUsedMismatch);
    }
    Ok(())
}

/// Perform validations over the block's gas usage.
/// Must be called only if the block has osaka activated
/// as specified in https://eips.ethereum.org/EIPS/eip-7825
fn verify_transaction_max_gas_limit(block: &Block) -> Result<(), InvalidBlockError> {
    for transaction in block.body.transactions.iter() {
        if transaction.gas_limit() > POST_OSAKA_GAS_LIMIT_CAP {
            return Err(InvalidBlockError::InvalidTransaction(format!(
                "Transaction gas limit exceeds maximum. Transaction hash: {}, transaction gas limit: {}",
                transaction.hash(),
                transaction.gas_limit()
            )));
        }
    }
    Ok(())
}

/// Calculates the blob gas required by a transaction.
pub fn get_total_blob_gas(tx: &EIP4844Transaction) -> u32 {
    GAS_PER_BLOB * tx.blob_versioned_hashes.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::H256;
    use crate::types::{
        Block, BlockBody, BlockHeader, ChainConfig, Frame, FrameMode, FrameTransaction, Transaction,
    };
    use bytes::Bytes;

    /// Minimal cancun-active ChainConfig: only cancun_time set (= 0), default
    /// blob schedule (max = 6 blobs per block).
    fn cancun_config() -> ChainConfig {
        ChainConfig {
            cancun_time: Some(0),
            ..Default::default()
        }
    }

    /// A minimal FrameTransaction with the given number of blob versioned hashes.
    fn frame_tx_with_blobs(n_blobs: usize) -> FrameTransaction {
        FrameTransaction {
            chain_id: 0,
            nonce: 0,
            sender: Default::default(),
            frames: vec![Frame {
                mode: FrameMode::Default as u8,
                flags: 0x00,
                target: None,
                gas_limit: 0,
                value: Default::default(),
                data: Bytes::new(),
            }],
            signatures: vec![],
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: Default::default(),
            blob_versioned_hashes: (0..n_blobs).map(|_| H256::zero()).collect(),
            ..Default::default()
        }
    }

    /// Build a minimal Block with the given transactions and blob_gas_used header
    /// value. timestamp = 1 so cancun_time = 0 is active.
    fn make_block(transactions: Vec<Transaction>, blob_gas_used: u64) -> Block {
        Block {
            header: BlockHeader {
                timestamp: 1,
                gas_limit: 30_000_000,
                blob_gas_used: Some(blob_gas_used),
                excess_blob_gas: Some(0),
                ..Default::default()
            },
            body: BlockBody {
                transactions,
                ommers: vec![],
                withdrawals: Some(vec![]),
            },
        }
    }

    // --- EIP-4844 blob gas accounting (regression guard) ---

    #[test]
    fn eip4844_blob_gas_counts_correctly() {
        let config = cancun_config();
        let tx = Transaction::EIP4844Transaction(EIP4844Transaction {
            blob_versioned_hashes: vec![H256::zero(), H256::zero()],
            ..Default::default()
        });
        let block = make_block(vec![tx], 2 * GAS_PER_BLOB as u64);
        assert!(verify_blob_gas_usage(&block, &config).is_ok());
    }

    #[test]
    fn eip4844_blob_gas_mismatch_fails() {
        let config = cancun_config();
        let tx = Transaction::EIP4844Transaction(EIP4844Transaction {
            blob_versioned_hashes: vec![H256::zero(), H256::zero()],
            ..Default::default()
        });
        // Header claims 0 but actual is 2 * GAS_PER_BLOB
        let block = make_block(vec![tx], 0);
        assert!(matches!(
            verify_blob_gas_usage(&block, &config),
            Err(InvalidBlockError::BlobGasUsedMismatch)
        ));
    }

    // --- EIP-8141 frame tx blob gas accounting ---

    #[test]
    fn frame_tx_blob_gas_counts_correctly() {
        let config = cancun_config();
        let tx = Transaction::FrameTransaction(frame_tx_with_blobs(2));
        let block = make_block(vec![tx], 2 * GAS_PER_BLOB as u64);
        assert!(verify_blob_gas_usage(&block, &config).is_ok());
    }

    #[test]
    fn frame_tx_blob_gas_mismatch_fails() {
        let config = cancun_config();
        let tx = Transaction::FrameTransaction(frame_tx_with_blobs(2));
        // Header claims 0 but actual is 2 * GAS_PER_BLOB
        let block = make_block(vec![tx], 0);
        assert!(matches!(
            verify_blob_gas_usage(&block, &config),
            Err(InvalidBlockError::BlobGasUsedMismatch)
        ));
    }

    #[test]
    fn mixed_eip4844_and_frame_tx_blobs_counted_together() {
        let config = cancun_config();
        let eip4844_tx = Transaction::EIP4844Transaction(EIP4844Transaction {
            blob_versioned_hashes: vec![H256::zero()],
            ..Default::default()
        });
        let frame_tx = Transaction::FrameTransaction(frame_tx_with_blobs(2));
        let expected_gas = 3 * GAS_PER_BLOB as u64; // 1 EIP-4844 + 2 frame
        let block = make_block(vec![eip4844_tx, frame_tx], expected_gas);
        assert!(verify_blob_gas_usage(&block, &config).is_ok());
    }
}
