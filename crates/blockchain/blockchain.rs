pub mod constants;
pub mod error;
pub mod fork_choice;
pub mod mempool;
pub mod payload;
mod smoke_test;

use error::{ChainError, InvalidBlockError};
use ethrex_common::constants::GAS_PER_BLOB;
use ethrex_common::types::requests::{compute_requests_hash, EncodedRequests, Requests};
use ethrex_common::types::{
    compute_receipts_root, validate_block_header, validate_cancun_header_fields,
    validate_prague_header_fields, validate_pre_cancun_header_fields, Block, BlockHash,
    BlockHeader, BlockNumber, ChainConfig, EIP4844Transaction, Receipt, Transaction,
};
use ethrex_common::H256;
use std::{ops::Div, time::Instant};

use ethrex_storage::error::StoreError;
use ethrex_storage::Store;
use ethrex_vm::backends::BlockExecutionResult;
use ethrex_vm::backends::EVM;
use fork_choice::apply_fork_choice;
use tracing::{error, info, warn};

//TODO: Implement a struct Chain or BlockChain to encapsulate
//functionality and canonical chain state and config

#[derive(Debug, Clone)]
pub struct Blockchain {
    pub vm: EVM,
    pub storage: Store,
}

impl Blockchain {
    pub fn new(evm: EVM, store: Store) -> Self {
        Self {
            vm: evm,
            storage: store,
        }
    }

    pub fn default_with_store(store: Store) -> Self {
        Self {
            vm: Default::default(),
            storage: store,
        }
    }

    pub fn add_block(&self, block: &Block) -> Result<(), ChainError> {
        info!(
            "Add block: Adding block {} with hash {:#x}.",
            block.header.number,
            block.hash()
        );
        let since = Instant::now();

        let block_hash = block.header.compute_block_hash();

        // Validate if it can be the new head and find the parent
        let Ok(parent_header) = find_parent_header(&block.header, &self.storage) else {
            // If the parent is not present, we store it as pending.
            self.storage.add_pending_block(block.clone())?;
            return Err(ChainError::ParentNotFound);
        };
        // let mut state = evm_state(self.storage.clone(), block.header.parent_hash);
        let chain_config = self.storage.get_chain_config()?;

        // Validate the block pre-execution
        validate_block(block, &parent_header, &chain_config)?;
        let BlockExecutionResult {
            receipts,
            requests,
            account_updates,
        } = self.vm.execute_block(block)?;

        validate_gas_used(&receipts, &block.header)?;

        // Apply the account updates over the last block's state and compute the new state root
        let new_state_root = self
            .storage
            .apply_account_updates(block.header.parent_hash, &account_updates)?
            .ok_or(ChainError::ParentStateNotFound)?;

        // Check state root matches the one in block header after execution
        validate_state_root(&block.header, new_state_root)?;

        // Check receipts root matches the one in block header after execution
        validate_receipts_root(&block.header, &receipts)?;

        // Processes requests from receipts, computes the requests_hash and compares it against the header
        validate_requests_hash(&block.header, &chain_config, &requests)?;

        store_block(&self.storage, block.clone())?;
        store_receipts(&self.storage, receipts, block_hash)?;

        let interval = Instant::now().duration_since(since).as_millis();
        if interval != 0 {
            let as_gigas = (block.header.gas_used as f64).div(10_f64.powf(9_f64));
            let throughput = (as_gigas) / (interval as f64) * 1000_f64;
            info!("[METRIC] BLOCK EXECUTION THROUGHPUT: {throughput} Gigagas/s TIME SPENT: {interval} msecs");
        }

        Ok(())
    }

    //TODO: Forkchoice Update shouldn't be part of this function
    pub fn import_blocks(&self, blocks: &Vec<Block>) {
        let size = blocks.len();
        for block in blocks {
            let hash = block.hash();
            info!(
                "Adding block {} with hash {:#x}.",
                block.header.number, hash
            );
            if let Err(error) = self.add_block(block) {
                warn!(
                    "Failed to add block {} with hash {:#x}: {}.",
                    block.header.number, hash, error
                );
            }
            if self
                .storage
                .update_latest_block_number(block.header.number)
                .is_err()
            {
                error!("Fatal: added block {} but could not update the block number -- aborting block import", block.header.number);
                break;
            };
            if self
                .storage
                .set_canonical_block(block.header.number, hash)
                .is_err()
            {
                error!(
                    "Fatal: added block {} but could not set it as canonical -- aborting block import",
                    block.header.number
                );
                break;
            };
        }
        if let Some(last_block) = blocks.last() {
            let hash = last_block.hash();
            match self.vm {
                EVM::LEVM => {
                    // We are allowing this not to unwrap so that tests can run even if block execution results in the wrong root hash with LEVM.
                    let _ = apply_fork_choice(&self.storage, hash, hash, hash);
                }
                EVM::REVM => {
                    apply_fork_choice(&self.storage, hash, hash, hash).unwrap();
                }
            }
        }
        info!("Added {size} blocks to blockchain");
    }
}

pub fn validate_requests_hash(
    header: &BlockHeader,
    chain_config: &ChainConfig,
    requests: &[Requests],
) -> Result<(), ChainError> {
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
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::RequestsHashMismatch,
        ));
    }

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
    let latest_block_number = storage.get_latest_block_number()?;
    if let Some(latest_valid_header) = storage.get_block_header(latest_block_number)? {
        let latest_valid_hash = latest_valid_header.compute_block_hash();
        return Ok(latest_valid_hash);
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
    chain_config: &ChainConfig,
) -> Result<(), ChainError> {
    // Verify initial header validity against parent
    validate_block_header(&block.header, parent_header).map_err(InvalidBlockError::from)?;

    if chain_config.is_prague_activated(block.header.timestamp) {
        validate_prague_header_fields(&block.header, parent_header)
            .map_err(InvalidBlockError::from)?;
        verify_blob_gas_usage(block, chain_config)?;
    } else if chain_config.is_cancun_activated(block.header.timestamp) {
        validate_cancun_header_fields(&block.header, parent_header)
            .map_err(InvalidBlockError::from)?;
        verify_blob_gas_usage(block, chain_config)?;
    } else {
        validate_pre_cancun_header_fields(&block.header).map_err(InvalidBlockError::from)?
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

// Perform validations over the block's blob gas usage.
// Must be called only if the block has cancun activated
fn verify_blob_gas_usage(block: &Block, config: &ChainConfig) -> Result<(), ChainError> {
    let mut blob_gas_used = 0_u64;
    let mut blobs_in_block = 0_u64;
    let max_blob_number_per_block = config
        .get_fork_blob_schedule(block.header.timestamp)
        .map(|schedule| schedule.max)
        .ok_or(ChainError::Custom("Provided block fork is invalid".into()))?;
    let max_blob_gas_per_block = max_blob_number_per_block * GAS_PER_BLOB;

    for transaction in block.body.transactions.iter() {
        if let Transaction::EIP4844Transaction(tx) = transaction {
            blob_gas_used += get_total_blob_gas(tx);
            blobs_in_block += tx.blob_versioned_hashes.len() as u64;
        }
    }
    if blob_gas_used > max_blob_gas_per_block {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::ExceededMaxBlobGasPerBlock,
        ));
    }
    if blobs_in_block > max_blob_number_per_block {
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
