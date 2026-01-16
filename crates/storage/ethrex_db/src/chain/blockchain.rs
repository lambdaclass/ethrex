//! Blockchain - manages blocks before finalization.
//!
//! This component handles the "hot" state that hasn't been finalized yet.
//! It supports parallel block creation from the same parent and Fork Choice Updates.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use ethereum_types::H256;
use thiserror::Error;

use super::block::{Block, BlockId};
use super::world_state::Account;
use crate::store::{PagedDb, DbError};

/// Blockchain errors.
#[derive(Error, Debug)]
pub enum BlockchainError {
    #[error("Block not found: {0:?}")]
    BlockNotFound(H256),
    #[error("Parent block not found: {0:?}")]
    ParentNotFound(H256),
    #[error("Block already exists: {0:?}")]
    BlockExists(H256),
    #[error("Invalid block number")]
    InvalidBlockNumber,
    #[error("Database error: {0}")]
    Database(#[from] DbError),
}

/// Result type for blockchain operations.
pub type Result<T> = std::result::Result<T, BlockchainError>;

/// Internal block state for committed blocks.
struct CommittedBlock {
    block: Block,
}

/// The blockchain manager.
///
/// Handles blocks that are not yet finalized, allowing:
/// - Parallel block creation from the same parent
/// - Fork choice updates (FCU)
/// - Finalization that flushes to PagedDb
pub struct Blockchain {
    /// The underlying database for finalized state.
    db: Arc<RwLock<PagedDb>>,
    /// Committed blocks by hash.
    blocks_by_hash: RwLock<HashMap<H256, CommittedBlock>>,
    /// Committed blocks by number (multiple blocks can have the same number).
    blocks_by_number: RwLock<HashMap<u64, Vec<H256>>>,
    /// The last finalized block number.
    last_finalized: RwLock<u64>,
    /// The last finalized block hash.
    last_finalized_hash: RwLock<H256>,
}

impl Blockchain {
    /// Creates a new blockchain with the given database.
    pub fn new(db: PagedDb) -> Self {
        let block_number = db.block_number() as u64;
        let block_hash = H256::from(db.block_hash());

        Self {
            db: Arc::new(RwLock::new(db)),
            blocks_by_hash: RwLock::new(HashMap::new()),
            blocks_by_number: RwLock::new(HashMap::new()),
            last_finalized: RwLock::new(block_number),
            last_finalized_hash: RwLock::new(block_hash),
        }
    }

    /// Returns the last finalized block number.
    pub fn last_finalized_number(&self) -> u64 {
        *self.last_finalized.read().unwrap()
    }

    /// Returns the last finalized block hash.
    pub fn last_finalized_hash(&self) -> H256 {
        *self.last_finalized_hash.read().unwrap()
    }

    /// Starts a new block on top of the given parent.
    ///
    /// This supports parallel block creation - multiple blocks can be created
    /// on top of the same parent concurrently.
    pub fn start_new(&self, parent_hash: H256, block_hash: H256, block_number: u64) -> Result<Block> {
        // Check if parent exists (either finalized or committed)
        let finalized_hash = *self.last_finalized_hash.read().unwrap();
        let finalized_number = *self.last_finalized.read().unwrap();

        if parent_hash == finalized_hash {
            // Building on top of finalized state
            if block_number != finalized_number + 1 {
                return Err(BlockchainError::InvalidBlockNumber);
            }
            return Ok(Block::new(block_number, block_hash, parent_hash));
        }

        // Check committed blocks
        let blocks = self.blocks_by_hash.read().unwrap();
        if let Some(parent) = blocks.get(&parent_hash) {
            if block_number != parent.block.number() + 1 {
                return Err(BlockchainError::InvalidBlockNumber);
            }
            // Note: In a full implementation, we'd clone or reference the parent state
            return Ok(Block::new(block_number, block_hash, parent_hash));
        }

        Err(BlockchainError::ParentNotFound(parent_hash))
    }

    /// Commits a block (makes it available for queries but not finalized).
    pub fn commit(&self, block: Block) -> Result<()> {
        let hash = block.hash();
        let number = block.number();

        let mut blocks_by_hash = self.blocks_by_hash.write().unwrap();
        let mut blocks_by_number = self.blocks_by_number.write().unwrap();

        // Check for duplicates
        if blocks_by_hash.contains_key(&hash) {
            return Err(BlockchainError::BlockExists(hash));
        }

        // Add to hash index
        blocks_by_hash.insert(hash, CommittedBlock { block });

        // Add to number index
        blocks_by_number
            .entry(number)
            .or_insert_with(Vec::new)
            .push(hash);

        Ok(())
    }

    /// Gets a committed block by hash.
    pub fn get_block(&self, hash: &H256) -> Option<BlockId> {
        let blocks = self.blocks_by_hash.read().unwrap();
        blocks.get(hash).map(|b| b.block.id)
    }

    /// Gets an account from the specified block.
    pub fn get_account(&self, block_hash: &H256, address: &H256) -> Option<Account> {
        let blocks = self.blocks_by_hash.read().unwrap();
        if let Some(committed) = blocks.get(block_hash) {
            return committed.block.get_account(address);
        }
        None
    }

    /// Finalizes blocks up to the given hash.
    ///
    /// This flushes the state to the underlying PagedDb and removes
    /// the finalized blocks from memory.
    pub fn finalize(&self, block_hash: H256) -> Result<()> {
        // Find the block
        let block_number = {
            let blocks = self.blocks_by_hash.read().unwrap();
            let block = blocks.get(&block_hash)
                .ok_or(BlockchainError::BlockNotFound(block_hash))?;
            block.block.number()
        };

        // Get the chain of blocks to finalize
        let mut to_finalize = Vec::new();
        let mut current_hash = block_hash;
        let finalized_hash = *self.last_finalized_hash.read().unwrap();

        {
            let blocks = self.blocks_by_hash.read().unwrap();
            while current_hash != finalized_hash {
                let block = blocks.get(&current_hash)
                    .ok_or(BlockchainError::BlockNotFound(current_hash))?;
                to_finalize.push(current_hash);
                current_hash = block.block.parent_hash;
            }
        }

        // Reverse to process in order
        to_finalize.reverse();

        // Apply blocks to database
        {
            let mut db = self.db.write().unwrap();
            for hash in &to_finalize {
                let blocks = self.blocks_by_hash.read().unwrap();
                let block = &blocks.get(hash).unwrap().block;

                let mut batch = db.begin_batch();

                // In a full implementation, we'd apply the block's state changes here
                batch.set_metadata(
                    block.number() as u32,
                    block.hash().as_fixed_bytes(),
                );

                batch.commit(crate::store::CommitOptions::FlushDataOnly)?;
            }
        }

        // Remove finalized blocks from memory
        {
            let mut blocks_by_hash = self.blocks_by_hash.write().unwrap();
            let mut blocks_by_number = self.blocks_by_number.write().unwrap();

            for hash in &to_finalize {
                blocks_by_hash.remove(hash);
            }

            // Clean up number index
            let finalized_num = *self.last_finalized.read().unwrap();
            for num in finalized_num + 1..=block_number {
                if let Some(hashes) = blocks_by_number.get_mut(&num) {
                    hashes.retain(|h| !to_finalize.contains(h));
                    if hashes.is_empty() {
                        blocks_by_number.remove(&num);
                    }
                }
            }
        }

        // Update finalized state
        *self.last_finalized.write().unwrap() = block_number;
        *self.last_finalized_hash.write().unwrap() = block_hash;

        Ok(())
    }

    /// Returns the number of committed (non-finalized) blocks.
    pub fn committed_count(&self) -> usize {
        self.blocks_by_hash.read().unwrap().len()
    }

    /// Handles a Fork Choice Update.
    ///
    /// This sets the canonical head and optionally finalizes blocks.
    pub fn fork_choice_update(
        &self,
        head_hash: H256,
        _safe_hash: Option<H256>,
        finalized_hash: Option<H256>,
    ) -> Result<()> {
        // Verify head exists
        {
            let blocks = self.blocks_by_hash.read().unwrap();
            if !blocks.contains_key(&head_hash) {
                // Check if it's the finalized block
                if head_hash != *self.last_finalized_hash.read().unwrap() {
                    return Err(BlockchainError::BlockNotFound(head_hash));
                }
            }
        }

        // Finalize if requested
        if let Some(finalized) = finalized_hash {
            if finalized != *self.last_finalized_hash.read().unwrap() {
                self.finalize(finalized)?;
            }
        }

        Ok(())
    }
}

use super::world_state::ReadOnlyWorldState;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::PagedDb;

    fn create_test_blockchain() -> Blockchain {
        let db = PagedDb::in_memory(1000).unwrap();
        Blockchain::new(db)
    }

    #[test]
    fn test_start_new_block() {
        let blockchain = create_test_blockchain();
        let parent_hash = blockchain.last_finalized_hash();

        let block = blockchain.start_new(
            parent_hash,
            H256::repeat_byte(0x01),
            1,
        ).unwrap();

        assert_eq!(block.number(), 1);
        assert_eq!(block.parent_hash, parent_hash);
    }

    #[test]
    fn test_commit_block() {
        let blockchain = create_test_blockchain();
        let parent_hash = blockchain.last_finalized_hash();

        let block = blockchain.start_new(
            parent_hash,
            H256::repeat_byte(0x01),
            1,
        ).unwrap();

        blockchain.commit(block).unwrap();
        assert_eq!(blockchain.committed_count(), 1);
    }

    #[test]
    fn test_parallel_blocks() {
        let blockchain = create_test_blockchain();
        let parent_hash = blockchain.last_finalized_hash();

        // Create two blocks with the same parent
        let block1 = blockchain.start_new(
            parent_hash,
            H256::repeat_byte(0x01),
            1,
        ).unwrap();

        let block2 = blockchain.start_new(
            parent_hash,
            H256::repeat_byte(0x02),
            1,
        ).unwrap();

        blockchain.commit(block1).unwrap();
        blockchain.commit(block2).unwrap();

        assert_eq!(blockchain.committed_count(), 2);
    }

    #[test]
    fn test_finalize() {
        let blockchain = create_test_blockchain();
        let parent_hash = blockchain.last_finalized_hash();

        let block = blockchain.start_new(
            parent_hash,
            H256::repeat_byte(0x01),
            1,
        ).unwrap();
        let block_hash = block.hash();

        blockchain.commit(block).unwrap();
        assert_eq!(blockchain.committed_count(), 1);

        blockchain.finalize(block_hash).unwrap();

        assert_eq!(blockchain.committed_count(), 0);
        assert_eq!(blockchain.last_finalized_number(), 1);
        assert_eq!(blockchain.last_finalized_hash(), block_hash);
    }
}
