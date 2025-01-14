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
use ethrex_storage::{AccountUpdate, Store};
use ethrex_vm::revm;
use ethrex_vm::{EvmState, EVM};

//TODO: Implement a struct Chain or BlockChain to encapsulate
//functionality and canonical chain state and config
#[derive(Debug, Clone)]
pub struct BlockChain {
    store: Store,
    evm: EVM,
}

impl BlockChain {
    pub fn new(storage: Store, evm: EVM) -> Self {
        Self {
            store: storage,
            evm,
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn evm(&self) -> &EVM {
        &self.evm
    }

    /// Adds a new block to the store. It may or may not be canonical, as long as its ancestry links
    /// with the canonical chain and its parent's post-state is calculated. It doesn't modify the
    /// canonical chain/head. Fork choice needs to be updated for that in a separate step.
    ///
    /// Performs pre and post execution validation, and updates the database with the post state.
    pub fn add_block(&self, block: &Block) -> Result<(), ChainError> {
        let block_hash = block.header.compute_block_hash();

        // Validate if it can be the new head and find the parent
        let Ok(parent_header) = self.find_parent_header(&block.header) else {
            // If the parent is not present, we store it as pending.
            self.store.add_pending_block(block.clone())?;
            return Err(ChainError::ParentNotFound);
        };
        let mut state = revm::evm_state(self.store.clone(), block.header.parent_hash);

        // Validate the block pre-execution
        Self::validate_block(block, &parent_header, &state)?;
        let (receipts, account_updates): (Vec<Receipt>, Vec<AccountUpdate>) =
            ethrex_vm::execute_block(block, &mut state)?;

        Self::validate_gas_used(&receipts, &block.header)?;

        // Apply the account updates over the last block's state and compute the new state root
        let new_state_root = state
            .database()
            .ok_or(ChainError::StoreError(StoreError::MissingStore))?
            .apply_account_updates(block.header.parent_hash, &account_updates)?
            .ok_or(ChainError::ParentStateNotFound)?;

        // Check state root matches the one in block header after execution
        Self::validate_state_root(&block.header, new_state_root)?;

        // Check receipts root matches the one in block header after execution
        Self::validate_receipts_root(&block.header, &receipts)?;

        self.store_block(block.clone())?;
        self.store_receipts(receipts, block_hash)?;

        Ok(())
    }

    /// Stores block and header in the database
    pub fn store_block(&self, block: Block) -> Result<(), ChainError> {
        self.store.add_block(block)?;
        Ok(())
    }

    pub fn store_receipts(
        &self,
        receipts: Vec<Receipt>,
        block_hash: BlockHash,
    ) -> Result<(), ChainError> {
        self.store.add_receipts(block_hash, receipts)?;
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
    pub fn latest_canonical_block_hash(&self) -> Result<H256, ChainError> {
        let latest_block_number = self.store.get_latest_block_number()?;
        if let Some(latest_valid_header) = self.store.get_block_header(latest_block_number)? {
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
        &self,
        block_header: &BlockHeader,
    ) -> Result<BlockHeader, ChainError> {
        match self
            .store
            .get_block_header_by_hash(block_header.parent_hash)?
        {
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
        let spec = revm::spec_id(
            &state.chain_config().map_err(ChainError::from)?,
            block.header.timestamp,
        );

        // Verify initial header validity against parent
        validate_block_header(&block.header, parent_header).map_err(InvalidBlockError::from)?;

        match spec {
            revm::SpecId::CANCUN => validate_cancun_header_fields(&block.header, parent_header)
                .map_err(InvalidBlockError::from)?,
            _other_specs => {
                validate_no_cancun_header_fields(&block.header).map_err(InvalidBlockError::from)?
            }
        };

        if spec == revm::SpecId::CANCUN {
            Self::verify_blob_gas_usage(block)?
        }
        Ok(())
    }

    pub fn is_canonical(
        &self,
        block_number: BlockNumber,
        block_hash: BlockHash,
    ) -> Result<bool, StoreError> {
        match self.store.get_canonical_block_hash(block_number)? {
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
}

/// Calculates the blob gas required by a transaction
fn get_total_blob_gas(tx: &EIP4844Transaction) -> u64 {
    GAS_PER_BLOB * tx.blob_versioned_hashes.len() as u64
}

#[cfg(test)]
mod tests {}
