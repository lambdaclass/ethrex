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
use ethrex_crypto::Crypto;
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
        if chain_config.is_osaka_activated(block.header.timestamp)
            && !chain_config.is_amsterdam_activated(block.header.timestamp)
        {
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
    crypto: &dyn Crypto,
) -> Result<(), InvalidBlockError> {
    let receipts_root = compute_receipts_root(receipts, crypto);

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
    indices: impl Iterator<Item = u32>,
    max_valid_index: u32,
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

/// Validates that all indices in the header BAL are within valid bounds (Amsterdam+).
/// This is a subset of the full hash check — used in the parallel execution path
/// where we have the header BAL but do not build a new BAL during execution.
/// Per EIP-7928: valid indices are 0 (pre-exec) through len(transactions)+1 (post-exec).
pub fn validate_header_bal_indices(
    bal: &crate::types::block_access_list::BlockAccessList,
    transaction_count: usize,
) -> Result<(), InvalidBlockError> {
    let max_valid_index = u32::try_from(transaction_count + 1).unwrap_or(u32::MAX);

    for account in bal.accounts() {
        validate_bal_indices(
            account
                .storage_changes
                .iter()
                .flat_map(|slot| slot.slot_changes.iter().map(|c| c.block_access_index)),
            max_valid_index,
        )?;
        validate_bal_indices(
            account.balance_changes.iter().map(|c| c.block_access_index),
            max_valid_index,
        )?;
        validate_bal_indices(
            account.nonce_changes.iter().map(|c| c.block_access_index),
            max_valid_index,
        )?;
        validate_bal_indices(
            account.code_changes.iter().map(|c| c.block_access_index),
            max_valid_index,
        )?;
    }
    Ok(())
}

/// Validates that the block access list hash matches the block header (Amsterdam+).
/// Also validates that all BlockAccessIndex values are within valid bounds per EIP-7928,
/// and that the BAL size does not exceed the gas-derived limit.
pub fn validate_block_access_list_hash(
    header: &BlockHeader,
    chain_config: &ChainConfig,
    computed_bal: &crate::types::block_access_list::BlockAccessList,
    transaction_count: usize,
) -> Result<(), InvalidBlockError> {
    use crate::constants::BAL_ITEM_COST;

    // BAL validation only applies to Amsterdam+ forks
    if !chain_config.is_amsterdam_activated(header.timestamp) {
        return Ok(());
    }

    // Per EIP-7928: "Invalidate block if access list...contains indices exceeding len(transactions) + 1"
    // Index semantics: 0=pre-exec, 1..n=tx indices, n+1=post-exec (withdrawals)
    let max_valid_index = u32::try_from(transaction_count + 1).unwrap_or(u32::MAX);

    // Validate all indices and compute item count in a single pass over the BAL.
    let mut bal_items: u64 = 0;
    for account in computed_bal.accounts() {
        bal_items += 1; // address
        bal_items += account.storage_reads.len() as u64;
        bal_items += account.storage_changes.len() as u64;

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

    // EIP-7928 size cap: bal_items <= gas_limit / GAS_BLOCK_ACCESS_LIST_ITEM
    let max_items = header.gas_limit / BAL_ITEM_COST;
    if bal_items > max_items {
        return Err(InvalidBlockError::BlockAccessListSizeExceeded {
            items: bal_items,
            max_items,
        });
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
/// Per EIP-7928: bal_items <= block_gas_limit // GAS_BLOCK_ACCESS_LIST_ITEM
///
/// Prefer using [`validate_block_access_list_hash`] when both hash and size validation are needed,
/// as it performs both checks in a single pass over the BAL.
pub fn validate_block_access_list_size(
    header: &BlockHeader,
    chain_config: &ChainConfig,
    computed_bal: &crate::types::block_access_list::BlockAccessList,
) -> Result<(), InvalidBlockError> {
    use crate::constants::BAL_ITEM_COST;

    if !chain_config.is_amsterdam_activated(header.timestamp) {
        return Ok(());
    }

    let bal_items = computed_bal.item_count();
    let max_items = header.gas_limit / BAL_ITEM_COST;

    if bal_items > max_items {
        return Err(InvalidBlockError::BlockAccessListSizeExceeded {
            items: bal_items,
            max_items,
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

/// EIP-7805 (FOCIL) block-import side-channel. Carries the inclusion list
/// from `engine_newPayloadV6` into the block-validation path so that the
/// satisfaction algorithm (in `ethrex_blockchain::inclusion_list_validator`)
/// can run after block execution.
///
/// Per Decision 9 in `design.md`: the IL is *not* part of `Block` itself
/// (FOCIL is a CL/EL hybrid; the block on-chain has no notion of an IL).
/// This struct is request-scoped, populated only by V6 callers, and dropped
/// after validation. Non-V6 callers pass `BlockValidationContext::empty()`.
#[derive(Debug, Clone, Default)]
pub struct BlockValidationContext {
    /// RLP-decoded IL transactions from `engine_newPayloadV6`'s
    /// `inclusionListTransactions` parameter. `None` for non-V6 callers
    /// or empty ILs (treated as no-op by the satisfaction validator).
    pub inclusion_list: Option<Vec<crate::types::Transaction>>,
}

impl BlockValidationContext {
    /// Construct a context with no inclusion list. Use this for non-V6
    /// callers (V1-V5 newPayload, P2P sync, snap sync, devnet imports).
    /// Callers passing this guarantee the satisfaction check is a no-op.
    pub fn empty() -> Self {
        Self {
            inclusion_list: None,
        }
    }

    /// Construct a context from a V6 `inclusionListTransactions` parameter.
    /// Empty IL collapses to `None` so the satisfaction validator skips
    /// initialization.
    pub fn with_inclusion_list(il: Vec<crate::types::Transaction>) -> Self {
        Self {
            inclusion_list: if il.is_empty() { None } else { Some(il) },
        }
    }
}
