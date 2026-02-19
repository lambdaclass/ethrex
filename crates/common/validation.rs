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
/// Note that this doesn't validate that the transactions or withdrawals root of the header matches the body
/// contents, since we assume the caller already did it. And, in any case, that wouldn't invalidate the block header.
pub fn validate_block(
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

/// Validates that the block access list does not exceed the maximum allowed size (Amsterdam+).
/// Per EIP-7928: bal_items * ITEM_COST <= available_gas + system_allowance
pub fn validate_block_access_list_size(
    header: &BlockHeader,
    chain_config: &ChainConfig,
    computed_bal: &crate::types::block_access_list::BlockAccessList,
    transaction_count: usize,
) -> Result<(), InvalidBlockError> {
    use crate::constants::{
        BAL_ITEM_COST, BAL_TX_BASE_COST, MAX_CONSOLIDATION_REQUESTS_PER_BLOCK,
        MAX_WITHDRAWAL_REQUESTS_PER_BLOCK,
    };

    if !chain_config.is_amsterdam_activated(header.timestamp) {
        return Ok(());
    }

    let bal_items = computed_bal.item_count();
    let tx_count = transaction_count as u64;

    let available_gas = header
        .gas_limit
        .saturating_sub(tx_count.saturating_mul(BAL_TX_BASE_COST));

    let system_allowance = (15
        + 3 * (MAX_WITHDRAWAL_REQUESTS_PER_BLOCK + MAX_CONSOLIDATION_REQUESTS_PER_BLOCK))
        * BAL_ITEM_COST;

    let total_cost = bal_items.saturating_mul(BAL_ITEM_COST);
    let max_allowed = available_gas.saturating_add(system_allowance);

    if total_cost > max_allowed {
        return Err(InvalidBlockError::BlockAccessListSizeExceeded {
            items: bal_items,
            total_cost,
            max_allowed,
        });
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
        if let crate::types::Transaction::EIP4844Transaction(tx) = transaction {
            blob_gas_used += get_total_blob_gas(tx);
            blobs_in_block += tx.blob_versioned_hashes.len() as u32;
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
