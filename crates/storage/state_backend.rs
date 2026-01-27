//! # State Storage Backend
//!
//! This module provides a specialized trait for state storage operations
//! (accounts and storage slots), designed to be implemented by ethrex-db.
//!
//! The trait is separate from the general StorageBackend because:
//! - State storage has different access patterns (trie-based vs key-value)
//! - ethrex-db provides optimized state storage with its own trie implementation
//! - This allows a hybrid approach where blocks/receipts use RocksDB and state uses ethrex-db

use ethereum_types::{Address, H256, U256};
use std::fmt::Debug;

use crate::error::StoreError;

/// Account state as stored in the state trie.
///
/// This is a simplified view of an account - the full AccountInfo includes
/// additional fields that are computed (like storage_root).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountState {
    pub nonce: u64,
    pub balance: U256,
    pub code_hash: H256,
    pub storage_root: H256,
}

impl AccountState {
    /// Empty account code hash (keccak256 of empty bytes).
    pub const EMPTY_CODE_HASH: H256 = H256([
        0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03,
        0xc0, 0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85,
        0xa4, 0x70,
    ]);

    /// Empty trie root (keccak256 of RLP-encoded empty string).
    pub const EMPTY_STORAGE_ROOT: H256 = H256([
        0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0, 0xf8,
        0x6e, 0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5, 0xe3, 0x63,
        0xb4, 0x21,
    ]);

    /// Creates an empty account.
    pub fn empty() -> Self {
        Self {
            nonce: 0,
            balance: U256::zero(),
            code_hash: Self::EMPTY_CODE_HASH,
            storage_root: Self::EMPTY_STORAGE_ROOT,
        }
    }

    /// Checks if this is an empty account.
    pub fn is_empty(&self) -> bool {
        self.nonce == 0
            && self.balance.is_zero()
            && self.code_hash == Self::EMPTY_CODE_HASH
            && self.storage_root == Self::EMPTY_STORAGE_ROOT
    }
}

/// State update batch for atomic state changes.
///
/// Collects all state changes for a block and applies them atomically.
#[derive(Clone, Debug, Default)]
pub struct StateUpdate {
    /// Account changes: (address_hash, Option<AccountState>)
    /// None means delete the account
    pub account_changes: Vec<(H256, Option<AccountState>)>,

    /// Storage changes: (address_hash, Vec<(slot_hash, value)>)
    /// Zero value means delete the slot
    pub storage_changes: Vec<(H256, Vec<(H256, U256)>)>,
}

impl StateUpdate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_account(&mut self, address_hash: H256, account: AccountState) {
        self.account_changes.push((address_hash, Some(account)));
    }

    pub fn delete_account(&mut self, address_hash: H256) {
        self.account_changes.push((address_hash, None));
    }

    pub fn set_storage(&mut self, address_hash: H256, slot_hash: H256, value: U256) {
        // Find existing entry for this address or create new one
        if let Some(entry) = self
            .storage_changes
            .iter_mut()
            .find(|(addr, _)| *addr == address_hash)
        {
            entry.1.push((slot_hash, value));
        } else {
            self.storage_changes
                .push((address_hash, vec![(slot_hash, value)]));
        }
    }
}

/// Trait for state-specific storage operations.
///
/// This is the boundary between ethrex storage and ethrex-db.
/// Implementations provide account/storage trie operations with
/// optimized state management.
pub trait StateStorageBackend: Debug + Send + Sync {
    // =========================================================================
    // Account Operations
    // =========================================================================

    /// Gets an account by its hashed address.
    fn get_account(&self, address_hash: &H256) -> Result<Option<AccountState>, StoreError>;

    /// Gets an account by its address (will hash internally).
    fn get_account_by_address(&self, address: &Address) -> Result<Option<AccountState>, StoreError>;

    /// Sets an account by its hashed address.
    fn set_account(&mut self, address_hash: H256, account: AccountState) -> Result<(), StoreError>;

    /// Deletes an account by its hashed address.
    fn delete_account(&mut self, address_hash: &H256) -> Result<(), StoreError>;

    // =========================================================================
    // Storage Operations
    // =========================================================================

    /// Gets a storage value by hashed address and hashed slot.
    fn get_storage(&self, address_hash: &H256, slot_hash: &H256) -> Result<Option<U256>, StoreError>;

    /// Sets a storage value by hashed address and hashed slot.
    /// Setting to zero deletes the slot.
    fn set_storage(
        &mut self,
        address_hash: H256,
        slot_hash: H256,
        value: U256,
    ) -> Result<(), StoreError>;

    // =========================================================================
    // State Root
    // =========================================================================

    /// Returns the current state root.
    fn state_root(&self) -> Result<H256, StoreError>;

    /// Computes the state root without committing changes.
    fn compute_state_root(&mut self) -> Result<H256, StoreError>;

    // =========================================================================
    // Batch Operations
    // =========================================================================

    /// Applies a state update batch and returns the new state root.
    fn apply_state_update(&mut self, update: StateUpdate) -> Result<H256, StoreError>;

    /// Commits all pending changes to storage.
    fn commit(&mut self) -> Result<(), StoreError>;

    // =========================================================================
    // Proof Generation
    // =========================================================================

    /// Gets a Merkle proof for an account.
    fn get_account_proof(&self, address: &Address) -> Result<Vec<Vec<u8>>, StoreError>;

    /// Gets Merkle proofs for storage slots of an account.
    fn get_storage_proofs(
        &self,
        address: &Address,
        slots: &[H256],
    ) -> Result<Vec<Vec<Vec<u8>>>, StoreError>;

    // =========================================================================
    // Iteration (for snap sync)
    // =========================================================================

    /// Returns an iterator over accounts starting from the given hash.
    fn iter_accounts_from(
        &self,
        start: &H256,
    ) -> Result<Box<dyn Iterator<Item = (H256, AccountState)> + '_>, StoreError>;

    /// Returns an iterator over storage slots for an account starting from the given hash.
    fn iter_storage_from(
        &self,
        address_hash: &H256,
        start: &H256,
    ) -> Result<Box<dyn Iterator<Item = (H256, U256)> + '_>, StoreError>;

    // =========================================================================
    // Snap Sync Support
    // =========================================================================

    /// Sets multiple accounts in batch (optimized for snap sync).
    fn set_accounts_batch(
        &mut self,
        accounts: Vec<(H256, AccountState)>,
    ) -> Result<(), StoreError>;

    /// Sets multiple storage slots in batch (optimized for snap sync).
    fn set_storage_batch(
        &mut self,
        address_hash: H256,
        slots: Vec<(H256, U256)>,
    ) -> Result<(), StoreError>;

    /// Flushes storage tries to compute their roots and free memory.
    /// Returns the number of storage tries flushed.
    fn flush_storage_tries(&mut self) -> Result<usize, StoreError>;

    /// Persists current state as a checkpoint (for incremental snap sync).
    fn persist_checkpoint(&mut self, block_number: u64, block_hash: H256) -> Result<(), StoreError>;

    /// Finalizes state persistence (called at end of snap sync).
    fn persist_final(&mut self, block_number: u64, block_hash: H256) -> Result<(), StoreError>;
}
