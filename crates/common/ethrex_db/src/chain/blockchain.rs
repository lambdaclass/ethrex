//! Blockchain - manages blocks before finalization.
//!
//! This component handles the "hot" state that hasn't been finalized yet.
//! It supports parallel block creation from the same parent and Fork Choice Updates.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use primitive_types::{H256, U256};
use thiserror::Error;

use super::block::{Block, BlockId};
use super::world_state::Account;
use crate::store::{PagedDb, DbError, PagedStateTrie, AccountData, CommitOptions};

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
    /// The state trie for finalized state.
    state_trie: RwLock<PagedStateTrie>,
    /// Committed blocks by hash.
    blocks_by_hash: RwLock<HashMap<H256, CommittedBlock>>,
    /// Committed blocks by number (multiple blocks can have the same number).
    blocks_by_number: RwLock<HashMap<u64, Vec<H256>>>,
    /// The last finalized block number.
    last_finalized: RwLock<u64>,
    /// The last finalized block hash.
    last_finalized_hash: RwLock<H256>,
}

/// Converts chain Account to trie AccountData.
fn account_to_data(account: &Account) -> AccountData {
    AccountData {
        nonce: account.nonce,
        balance: account.balance.to_big_endian(),
        storage_root: *account.storage_root.as_fixed_bytes(),
        code_hash: *account.code_hash.as_fixed_bytes(),
    }
}

/// Converts trie AccountData to chain Account.
fn data_to_account(data: &AccountData) -> Account {
    Account {
        nonce: data.nonce,
        balance: U256::from_big_endian(&data.balance),
        code_hash: H256::from(data.code_hash),
        storage_root: H256::from(data.storage_root),
    }
}

impl Blockchain {
    /// Creates a new blockchain with the given database.
    pub fn new(db: PagedDb) -> Self {
        let block_number = db.block_number() as u64;
        let block_hash = H256::from(db.block_hash());

        // Load existing state trie if present
        let state_root = db.begin_read_only().state_root();
        let state_trie = if state_root.is_null() {
            PagedStateTrie::new()
        } else {
            PagedStateTrie::load(&db, state_root).unwrap_or_else(|_| PagedStateTrie::new())
        };

        Self {
            db: Arc::new(RwLock::new(db)),
            state_trie: RwLock::new(state_trie),
            blocks_by_hash: RwLock::new(HashMap::new()),
            blocks_by_number: RwLock::new(HashMap::new()),
            last_finalized: RwLock::new(block_number),
            last_finalized_hash: RwLock::new(block_hash),
        }
    }

    /// Creates a new blockchain with the given database and pre-populated state trie.
    ///
    /// Used by snap sync to initialize the blockchain with a trie that was
    /// built during the snap sync process.
    pub fn new_with_state_trie(db: PagedDb, state_trie: PagedStateTrie) -> Self {
        let block_number = db.block_number() as u64;
        let block_hash = H256::from(db.block_hash());

        Self {
            db: Arc::new(RwLock::new(db)),
            state_trie: RwLock::new(state_trie),
            blocks_by_hash: RwLock::new(HashMap::new()),
            blocks_by_number: RwLock::new(HashMap::new()),
            last_finalized: RwLock::new(block_number),
            last_finalized_hash: RwLock::new(block_hash),
        }
    }

    /// Replaces the state trie with a new one.
    ///
    /// Used by snap sync to set the trie after it has been populated.
    /// WARNING: This will discard any uncommitted blocks and their state.
    pub fn set_state_trie(&self, state_trie: PagedStateTrie) {
        // Clear any uncommitted blocks since their state is no longer valid
        self.blocks_by_hash.write().unwrap().clear();
        self.blocks_by_number.write().unwrap().clear();

        // Replace the state trie
        *self.state_trie.write().unwrap() = state_trie;
    }

    /// Persists the current state trie to the database.
    ///
    /// Used by snap sync after populating the state trie to persist it to disk.
    /// This also updates the block metadata to the given block number and hash.
    pub fn persist_state_trie(&self, block_number: u64, block_hash: H256) -> Result<()> {
        use crate::store::CommitOptions;

        let mut db = self.db.write().unwrap();
        let mut state_trie = self.state_trie.write().unwrap();
        let mut batch = db.begin_batch();

        // Save state trie and get the root address
        let state_root_addr = state_trie.save(&mut batch)?;
        batch.set_state_root(state_root_addr);

        // Update block metadata
        batch.set_metadata(
            block_number as u32,
            block_hash.as_fixed_bytes(),
        );

        batch.commit(CommitOptions::FlushDataOnly)?;

        // Update finalized state
        *self.last_finalized.write().unwrap() = block_number;
        *self.last_finalized_hash.write().unwrap() = block_hash;

        Ok(())
    }

    /// Persists the current state trie as a checkpoint without updating finalized block metadata.
    ///
    /// Used during snap sync to save incremental progress. Unlike `persist_state_trie`,
    /// this does not update the finalized block number/hash in memory, allowing the sync
    /// to resume from the checkpoint even if interrupted. The block metadata is saved
    /// to the database but the in-memory state is not updated until full finalization.
    pub fn persist_state_trie_checkpoint(&self, block_number: u64, block_hash: H256) -> Result<()> {
        use crate::store::CommitOptions;

        let mut db = self.db.write().unwrap();
        let mut state_trie = self.state_trie.write().unwrap();
        let mut batch = db.begin_batch();

        // Save state trie and get the root address
        let state_root_addr = state_trie.save(&mut batch)?;
        batch.set_state_root(state_root_addr);

        // Update block metadata in DB only (not in memory)
        // This allows recovery from the checkpoint on restart
        batch.set_metadata(
            block_number as u32,
            block_hash.as_fixed_bytes(),
        );

        batch.commit(CommitOptions::FlushDataOnly)?;

        // Note: We intentionally do NOT update last_finalized or last_finalized_hash here.
        // This is a checkpoint, not a full finalization. The in-memory state will be
        // updated only when persist_state_trie is called at the end of snap sync.

        Ok(())
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

        // Special case: genesis block (block 0) with parent H256::zero()
        // on an empty blockchain (finalized at block 0, hash zero)
        if block_number == 0 && parent_hash == H256::zero() && finalized_hash == H256::zero() && finalized_number == 0 {
            return Ok(Block::new(block_number, block_hash, parent_hash));
        }

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

    /// Gets a storage value from the specified block.
    pub fn get_storage(&self, block_hash: &H256, address: &H256, key: &H256) -> Option<U256> {
        let blocks = self.blocks_by_hash.read().unwrap();
        if let Some(committed) = blocks.get(block_hash) {
            return committed.block.get_storage(address, key);
        }
        None
    }

    /// Finalizes blocks up to the given hash.
    ///
    /// This flushes the state to the underlying PagedDb and removes
    /// the finalized blocks from memory.
    pub fn finalize(&self, block_hash: H256) -> Result<()> {
        // If already finalized to this hash, nothing to do
        if block_hash == *self.last_finalized_hash.read().unwrap() {
            return Ok(());
        }

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

        // Apply blocks to state trie and database
        {
            let mut db = self.db.write().unwrap();
            let mut state_trie = self.state_trie.write().unwrap();

            // Collect all state changes from blocks in order
            for hash in &to_finalize {
                let blocks = self.blocks_by_hash.read().unwrap();
                let block = &blocks.get(hash).unwrap().block;

                // Apply account changes to state trie
                for (addr, account_opt) in block.account_changes() {
                    // Convert H256 address to [u8; 20] (take last 20 bytes)
                    let addr_bytes: [u8; 20] = addr.as_bytes()[12..32].try_into().unwrap();

                    match account_opt {
                        Some(account) => {
                            let account_data = account_to_data(account);
                            state_trie.set_account(&addr_bytes, account_data);
                        }
                        None => {
                            // For deletion, set to empty account
                            // (In a full implementation, we'd remove it from the trie)
                            state_trie.set_account(&addr_bytes, AccountData::empty());
                        }
                    }
                }

                // Apply storage changes to state trie
                for (addr, slots) in block.storage_changes() {
                    let addr_bytes: [u8; 20] = addr.as_bytes()[12..32].try_into().unwrap();
                    let storage = state_trie.storage_trie(&addr_bytes);

                    for (key, value) in slots {
                        // H256 keys are already big-endian
                        let slot: [u8; 32] = *key.as_fixed_bytes();

                        // U256 values need to be converted
                        let val = value.to_big_endian();

                        storage.set(&slot, val);
                    }
                }
            }

            // Save state trie to database
            let mut batch = db.begin_batch();

            // Get the final block for metadata
            let blocks = self.blocks_by_hash.read().unwrap();
            let final_block = &blocks.get(&block_hash).unwrap().block;

            // Save state trie and get the root address
            let state_root_addr = state_trie.save(&mut batch)?;
            batch.set_state_root(state_root_addr);

            // Update block metadata
            batch.set_metadata(
                final_block.number() as u32,
                final_block.hash().as_fixed_bytes(),
            );

            batch.commit(CommitOptions::FlushDataOnly)?;
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

    /// Returns the state root hash of finalized state.
    pub fn state_root(&self) -> [u8; 32] {
        self.state_trie.write().unwrap().root_hash()
    }

    /// Gets an account from finalized state.
    pub fn get_finalized_account(&self, address: &[u8; 20]) -> Option<Account> {
        let trie = self.state_trie.read().unwrap();
        trie.get_account(address).map(|data| data_to_account(&data))
    }

    /// Gets an account from finalized state using a pre-hashed address.
    /// Used by snap sync which already has hashed addresses.
    pub fn get_finalized_account_by_hash(&self, address_hash: &[u8; 32]) -> Option<Account> {
        let trie = self.state_trie.read().unwrap();
        trie.get_account_by_hash(address_hash).map(|data| data_to_account(&data))
    }

    /// Gets a storage value from finalized state using pre-hashed keys.
    pub fn get_finalized_storage_by_hash(&self, address_hash: &[u8; 32], slot_hash: &[u8; 32]) -> Option<U256> {
        let trie = self.state_trie.read().unwrap();
        trie.get_storage_by_hash(address_hash, slot_hash)
            .map(|bytes| U256::from_big_endian(&bytes))
    }

    /// Returns the number of committed (non-finalized) blocks.
    pub fn committed_count(&self) -> usize {
        self.blocks_by_hash.read().unwrap().len()
    }

    /// Sets the genesis block as the initial finalized state.
    ///
    /// This should be called once when initializing a new blockchain with genesis.
    /// It sets the last finalized block hash and number without requiring
    /// the block to be in hot storage first.
    pub fn set_genesis(&self, genesis_hash: H256, genesis_number: u64) {
        *self.last_finalized.write().unwrap() = genesis_number;
        *self.last_finalized_hash.write().unwrap() = genesis_hash;
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
