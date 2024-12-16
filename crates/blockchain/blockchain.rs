pub mod constants;
pub mod error;
pub mod fork_choice;
pub mod mempool;
pub mod payload;
mod smoke_test;

use constants::{GAS_PER_BLOB, MAX_BLOB_GAS_PER_BLOCK, MAX_BLOB_NUMBER_PER_BLOCK};
use error::{ChainError, InvalidBlockError};
use ethrex_core::types::{
    compute_receipts_root, validate_block_header, validate_cancun_header_fields,
    validate_no_cancun_header_fields, Block, BlockHash, BlockHeader, BlockNumber,
    EIP4844Transaction, Receipt, Transaction,
};
use ethrex_core::H256;

use ethrex_storage::error::StoreError;
use ethrex_storage::Store;
use ethrex_vm::{evm_state, execute_block, spec_id, EvmState, SpecId};

//TODO: Implement a struct Chain or BlockChain to encapsulate
//functionality and canonical chain state and config

/// Adds a new block to the store. It may or may not be canonical, as long as its ancestry links
/// with the canonical chain and its parent's post-state is calculated. It doesn't modify the
/// canonical chain/head. Fork choice needs to be updated for that in a separate step.
///
/// Performs pre and post execution validation, and updates the database with the post state.
#[cfg(not(feature = "levm"))]
pub fn add_block(block: &Block, storage: &Store) -> Result<(), ChainError> {
    use ethrex_vm::get_state_transitions;

    let block_hash = block.header.compute_block_hash();

    // Validate if it can be the new head and find the parent
    let Ok(parent_header) = find_parent_header(&block.header, storage) else {
        // If the parent is not present, we store it as pending.
        storage.add_pending_block(block.clone())?;
        return Err(ChainError::ParentNotFound);
    };
    let mut state = evm_state(storage.clone(), block.header.parent_hash);

    // Validate the block pre-execution
    validate_block(block, &parent_header, &state)?;

    let receipts = execute_block(block, &mut state)?;

    validate_gas_used(&receipts, &block.header)?;

    let account_updates = get_state_transitions(&mut state);

    // Apply the account updates over the last block's state and compute the new state root
    let new_state_root = state
        .database()
        .ok_or(ChainError::StoreError(StoreError::MissingStore))?
        .apply_account_updates(block.header.parent_hash, &account_updates)?
        .ok_or(ChainError::ParentStateNotFound)?;

    // Check state root matches the one in block header after execution
    validate_state_root(&block.header, new_state_root)?;

    // Check receipts root matches the one in block header after execution
    validate_receipts_root(&block.header, &receipts)?;

    store_block(storage, block.clone())?;
    store_receipts(storage, receipts, block_hash)?;

    Ok(())
}

/// Adds a new block to the store. It may or may not be canonical, as long as its ancestry links
/// with the canonical chain and its parent's post-state is calculated. It doesn't modify the
/// canonical chain/head. Fork choice needs to be updated for that in a separate step.
///
/// Performs pre and post execution validation, and updates the database with the post state.
#[cfg(feature = "levm")]
pub fn add_block(block: &Block, storage: &Store) -> Result<(), ChainError> {
    let block_hash = block.header.compute_block_hash();

    // Validate if it can be the new head and find the parent
    let Ok(parent_header) = find_parent_header(&block.header, storage) else {
        // If the parent is not present, we store it as pending.
        storage.add_pending_block(block.clone())?;
        return Err(ChainError::ParentNotFound);
    };
    let mut state = evm_state(storage.clone(), block.header.parent_hash);

    // Validate the block pre-execution
    validate_block(block, &parent_header, &state)?;

    let (receipts, account_updates) = execute_block(block, &mut state)?;

    // Note: these is commented because it is still being used in development.
    // dbg!(&account_updates);

    validate_gas_used(&receipts, &block.header)?;

    // Apply the account updates over the last block's state and compute the new state root
    let new_state_root = state
        .database()
        .ok_or(ChainError::StoreError(StoreError::MissingStore))?
        .apply_account_updates(block.header.parent_hash, &account_updates)?
        .ok_or(ChainError::ParentStateNotFound)?;

    // Check state root matches the one in block header after execution
    validate_state_root(&block.header, new_state_root)?;

    // Check receipts root matches the one in block header after execution
    validate_receipts_root(&block.header, &receipts)?;

    store_block(storage, block.clone())?;
    store_receipts(storage, receipts, block_hash)?;

    Ok(())
}

/// Stores block and header in the database
pub fn store_block(storage: &Store, block: Block) -> Result<(), ChainError> {
    storage.add_block(block)?;
    Ok(())
}

pub fn store_receipts(
    storage: &Store,
    receipts: Vec<Receipt>,
    block_hash: BlockHash,
) -> Result<(), ChainError> {
    storage.add_receipts(block_hash, receipts)?;
    Ok(())
}

/// Performs post-execution checks
pub fn validate_state_root(
    block_header: &BlockHeader,
    new_state_root: H256,
) -> Result<(), ChainError> {
    // Compare state root
    if new_state_root == block_header.state_root {
        Ok(())
    } else {
        Err(ChainError::InvalidBlock(
            InvalidBlockError::StateRootMismatch,
        ))
    }
}

pub fn validate_receipts_root(
    block_header: &BlockHeader,
    receipts: &[Receipt],
) -> Result<(), ChainError> {
    let receipts_root = compute_receipts_root(receipts);

    if receipts_root == block_header.receipts_root {
        Ok(())
    } else {
        Err(ChainError::InvalidBlock(
            InvalidBlockError::ReceiptsRootMismatch,
        ))
    }
}

// Returns the hash of the head of the canonical chain (the latest valid hash).
pub fn latest_canonical_block_hash(storage: &Store) -> Result<H256, ChainError> {
    if let Some(latest_block_number) = storage.get_latest_block_number()? {
        if let Some(latest_valid_header) = storage.get_block_header(latest_block_number)? {
            let latest_valid_hash = latest_valid_header.compute_block_hash();
            return Ok(latest_valid_hash);
        }
    }
    Err(ChainError::StoreError(StoreError::Custom(
        "Could not find latest valid hash".to_string(),
    )))
}

/// Validates if the provided block could be the new head of the chain, and returns the
/// parent_header in that case. If not found, the new block is saved as pending.
pub fn find_parent_header(
    block_header: &BlockHeader,
    storage: &Store,
) -> Result<BlockHeader, ChainError> {
    match storage.get_block_header_by_hash(block_header.parent_hash)? {
        Some(parent_header) => Ok(parent_header),
        None => Err(ChainError::ParentNotFound),
    }
}

/// Performs pre-execution validation of the block's header values in reference to the parent_header
/// Verifies that blob gas fields in the header are correct in reference to the block's body.
/// If a block passes this check, execution will still fail with execute_block when a transaction runs out of gas
pub fn validate_block(
    block: &Block,
    parent_header: &BlockHeader,
    state: &EvmState,
) -> Result<(), ChainError> {
    let spec = spec_id(
        &state.chain_config().map_err(ChainError::from)?,
        block.header.timestamp,
    );

    // Verify initial header validity against parent
    validate_block_header(&block.header, parent_header).map_err(InvalidBlockError::from)?;

    match spec {
        SpecId::CANCUN => validate_cancun_header_fields(&block.header, parent_header)
            .map_err(InvalidBlockError::from)?,
        _other_specs => {
            validate_no_cancun_header_fields(&block.header).map_err(InvalidBlockError::from)?
        }
    };

    if spec == SpecId::CANCUN {
        verify_blob_gas_usage(block)?
    }
    Ok(())
}

pub fn is_canonical(
    store: &Store,
    block_number: BlockNumber,
    block_hash: BlockHash,
) -> Result<bool, StoreError> {
    match store.get_canonical_block_hash(block_number)? {
        Some(hash) if hash == block_hash => Ok(true),
        _ => Ok(false),
    }
}

pub fn validate_gas_used(
    receipts: &[Receipt],
    block_header: &BlockHeader,
) -> Result<(), ChainError> {
    if let Some(last) = receipts.last() {
        // Note: This is commented because it is still being used in development.
        // dbg!(last.cumulative_gas_used);
        // dbg!(block_header.gas_used);
        if last.cumulative_gas_used != block_header.gas_used {
            return Err(ChainError::InvalidBlock(InvalidBlockError::GasUsedMismatch));
        }
    }
    Ok(())
}

fn verify_blob_gas_usage(block: &Block) -> Result<(), ChainError> {
    let mut blob_gas_used = 0_u64;
    let mut blobs_in_block = 0_u64;
    for transaction in block.body.transactions.iter() {
        if let Transaction::EIP4844Transaction(tx) = transaction {
            blob_gas_used += get_total_blob_gas(tx);
            blobs_in_block += tx.blob_versioned_hashes.len() as u64;
        }
    }
    if blob_gas_used > MAX_BLOB_GAS_PER_BLOCK {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::ExceededMaxBlobGasPerBlock,
        ));
    }
    if blobs_in_block > MAX_BLOB_NUMBER_PER_BLOCK {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::ExceededMaxBlobNumberPerBlock,
        ));
    }
    if block
        .header
        .blob_gas_used
        .is_some_and(|header_blob_gas_used| header_blob_gas_used != blob_gas_used)
    {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::BlobGasUsedMismatch,
        ));
    }
    Ok(())
}

/// Calculates the blob gas required by a transaction
fn get_total_blob_gas(tx: &EIP4844Transaction) -> u64 {
    GAS_PER_BLOB * tx.blob_versioned_hashes.len() as u64
}

#[cfg(test)]
mod tests {}
