//! BlockchainStateManager - Wraps ethrex_db's Blockchain for Ethereum state management.
//!
//! This module provides a bridge between ethrex's storage layer and ethrex_db's
//! high-performance state storage engine with hot/cold separation.

use std::path::Path;

use ethrex_common::{Address, H256, U256};
use ethrex_common::types::AccountInfo;

use ethrex_db::chain::{Account as EthrexDbAccount, Blockchain, BlockchainError, ReadOnlyWorldState, WorldState};
use ethrex_db::store::PagedDb;

use crate::error::StoreError;

/// Manages Ethereum state using ethrex_db's hot/cold storage architecture.
///
/// This provides:
/// - Hot storage for unfinalized blocks (Copy-on-Write semantics)
/// - Cold storage for finalized state (memory-mapped pages)
/// - Automatic state root computation via MerkleTrie
pub struct BlockchainStateManager {
    blockchain: Blockchain,
}

impl std::fmt::Debug for BlockchainStateManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockchainStateManager")
            .field("last_finalized", &self.blockchain.last_finalized_number())
            .finish()
    }
}

impl BlockchainStateManager {
    /// Creates a new state manager with in-memory storage.
    ///
    /// Useful for testing.
    pub fn in_memory(pages: u32) -> Result<Self, StoreError> {
        let db = PagedDb::in_memory(pages)
            .map_err(|e| StoreError::Custom(format!("Failed to create in-memory PagedDb: {}", e)))?;
        Ok(Self {
            blockchain: Blockchain::new(db),
        })
    }

    /// Creates a new state manager with file-backed storage.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let db = PagedDb::open(path)
            .map_err(|e| StoreError::Custom(format!("Failed to open PagedDb: {}", e)))?;
        Ok(Self {
            blockchain: Blockchain::new(db),
        })
    }

    /// Creates a new state manager with file-backed storage and initial size.
    pub fn open_with_size<P: AsRef<Path>>(path: P, initial_pages: u32) -> Result<Self, StoreError> {
        let db = PagedDb::open_with_size(path, initial_pages)
            .map_err(|e| StoreError::Custom(format!("Failed to open PagedDb: {}", e)))?;
        Ok(Self {
            blockchain: Blockchain::new(db),
        })
    }

    /// Returns the last finalized block number.
    pub fn last_finalized_number(&self) -> u64 {
        self.blockchain.last_finalized_number()
    }

    /// Returns the last finalized block hash.
    pub fn last_finalized_hash(&self) -> H256 {
        self.blockchain.last_finalized_hash()
    }

    /// Starts processing a new block.
    ///
    /// Creates a new block with Copy-on-Write state from the parent.
    pub fn start_block(
        &self,
        parent_hash: H256,
        block_hash: H256,
        block_number: u64,
    ) -> Result<BlockState, StoreError> {
        let block = self.blockchain.start_new(parent_hash, block_hash, block_number)
            .map_err(blockchain_error_to_store)?;
        Ok(BlockState { block })
    }

    /// Commits a block to hot storage.
    ///
    /// The block becomes queryable but is not yet finalized.
    pub fn commit_block(&self, block_state: BlockState) -> Result<(), StoreError> {
        self.blockchain.commit(block_state.block)
            .map_err(blockchain_error_to_store)
    }

    /// Finalizes blocks up to the given hash.
    ///
    /// State is flushed to cold storage (PagedDb) and removed from hot storage.
    pub fn finalize(&self, block_hash: H256) -> Result<(), StoreError> {
        self.blockchain.finalize(block_hash)
            .map_err(blockchain_error_to_store)
    }

    /// Handles a Fork Choice Update.
    pub fn fork_choice_update(
        &self,
        head_hash: H256,
        safe_hash: Option<H256>,
        finalized_hash: Option<H256>,
    ) -> Result<(), StoreError> {
        self.blockchain.fork_choice_update(head_hash, safe_hash, finalized_hash)
            .map_err(blockchain_error_to_store)
    }

    /// Gets an account from a committed (hot) block.
    pub fn get_account(&self, block_hash: &H256, address: &Address) -> Option<AccountInfo> {
        // Convert Address to H256 (pad with zeros on the left)
        let addr_h256 = address_to_h256(address);
        self.blockchain.get_account(block_hash, &addr_h256)
            .map(ethrex_db_account_to_info)
    }

    /// Gets an account from finalized (cold) state.
    pub fn get_finalized_account(&self, address: &Address) -> Option<AccountInfo> {
        let addr_bytes: [u8; 20] = address.0;
        self.blockchain.get_finalized_account(&addr_bytes)
            .map(ethrex_db_account_to_info)
    }

    /// Gets a storage value from a committed (hot) block.
    pub fn get_storage(&self, block_hash: &H256, address: &Address, slot: &H256) -> Option<U256> {
        let addr_h256 = address_to_h256(address);
        self.blockchain.get_storage(block_hash, &addr_h256, slot)
    }

    /// Gets the state root hash of finalized state.
    pub fn state_root(&self) -> H256 {
        H256::from(self.blockchain.state_root())
    }

    /// Returns the number of committed (non-finalized) blocks.
    pub fn committed_count(&self) -> usize {
        self.blockchain.committed_count()
    }

    /// Sets the genesis block as the initial finalized state.
    ///
    /// This must be called after initializing genesis state so that
    /// the state_manager knows the genesis hash for subsequent blocks.
    pub fn set_genesis(&self, genesis_hash: H256, genesis_number: u64) {
        self.blockchain.set_genesis(genesis_hash, genesis_number);
    }
}

/// Represents an in-progress block being built.
pub struct BlockState {
    block: ethrex_db::chain::Block,
}

impl BlockState {
    /// Sets an account in this block's state.
    pub fn set_account(&mut self, address: &Address, info: &AccountInfo) {
        let addr_h256 = address_to_h256(address);
        let account = info_to_ethrex_db_account(info);
        self.block.set_account(addr_h256, account);
    }

    /// Sets a storage slot in this block's state.
    pub fn set_storage(&mut self, address: &Address, slot: H256, value: U256) {
        let addr_h256 = address_to_h256(address);
        self.block.set_storage(addr_h256, slot, value);
    }

    /// Gets an account from this block's state (includes uncommitted changes).
    pub fn get_account(&self, address: &Address) -> Option<AccountInfo> {
        let addr_h256 = address_to_h256(address);
        self.block.get_account(&addr_h256)
            .map(ethrex_db_account_to_info)
    }

    /// Gets a storage value from this block's state.
    pub fn get_storage(&self, address: &Address, slot: &H256) -> Option<U256> {
        let addr_h256 = address_to_h256(address);
        self.block.get_storage(&addr_h256, slot)
    }

    /// Returns the block hash.
    pub fn hash(&self) -> H256 {
        self.block.hash()
    }

    /// Returns the block number.
    pub fn number(&self) -> u64 {
        self.block.number()
    }
}

// Conversion helpers

fn address_to_h256(address: &Address) -> H256 {
    let mut bytes = [0u8; 32];
    bytes[12..32].copy_from_slice(&address.0);
    H256::from(bytes)
}

fn ethrex_db_account_to_info(account: EthrexDbAccount) -> AccountInfo {
    AccountInfo {
        nonce: account.nonce,
        balance: account.balance,
        code_hash: account.code_hash,
    }
}

fn info_to_ethrex_db_account(info: &AccountInfo) -> EthrexDbAccount {
    EthrexDbAccount {
        nonce: info.nonce,
        balance: info.balance,
        code_hash: info.code_hash,
        storage_root: H256::zero(), // Will be computed on commit
    }
}

fn blockchain_error_to_store(e: BlockchainError) -> StoreError {
    StoreError::Custom(format!("Blockchain error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_state_manager() {
        let manager = BlockchainStateManager::in_memory(1000).unwrap();
        assert_eq!(manager.last_finalized_number(), 0);
    }

    #[test]
    fn test_block_lifecycle() {
        let manager = BlockchainStateManager::in_memory(1000).unwrap();
        let parent_hash = manager.last_finalized_hash();
        let block_hash = H256::repeat_byte(0x01);

        // Start a new block
        let mut block_state = manager.start_block(parent_hash, block_hash, 1).unwrap();

        // Set some account data
        let address = Address::repeat_byte(0xAB);
        let info = AccountInfo {
            nonce: 1,
            balance: U256::from(1000),
            code_hash: H256::zero(),
        };
        block_state.set_account(&address, &info);

        // Verify we can read it back
        let retrieved = block_state.get_account(&address).unwrap();
        assert_eq!(retrieved.nonce, 1);
        assert_eq!(retrieved.balance, U256::from(1000));

        // Commit the block
        manager.commit_block(block_state).unwrap();
        assert_eq!(manager.committed_count(), 1);

        // Finalize the block
        manager.finalize(block_hash).unwrap();
        assert_eq!(manager.committed_count(), 0);
        assert_eq!(manager.last_finalized_number(), 1);
        assert_eq!(manager.last_finalized_hash(), block_hash);

        // Verify finalized account is accessible
        let finalized = manager.get_finalized_account(&address).unwrap();
        assert_eq!(finalized.nonce, 1);
    }
}
