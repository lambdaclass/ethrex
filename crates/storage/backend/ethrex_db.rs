//! # ethrex-db Backend
//!
//! This module implements the StateStorageBackend trait using ethrex-db's
//! PagedDb and PagedStateTrie for optimized state storage.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │           EthrexDbBackend                   │
//! │  ┌───────────────────────────────────────┐  │
//! │  │  PagedDb (mmap storage)               │  │
//! │  └───────────────────────────────────────┘  │
//! │  ┌───────────────────────────────────────┐  │
//! │  │  PagedStateTrie                       │  │
//! │  │  ├── StateTrie (accounts)             │  │
//! │  │  └── StorageTrie (per-account slots)  │  │
//! │  └───────────────────────────────────────┘  │
//! └─────────────────────────────────────────────┘
//! ```
//!
//! ## Features
//!
//! - Memory-mapped page-based storage (4KB pages)
//! - Optimized batch operations for snap sync
//! - Copy-on-Write concurrency
//! - Automatic storage root computation
//! - Witness generation for zkVM proving

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use ethereum_types::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;

use ethrex_db::store::{AccountData, CommitOptions, DbError, PagedDb, PagedStateTrie};

use crate::error::StoreError;
use crate::state_backend::{AccountState, StateStorageBackend, StateUpdate};

/// Converts ethrex-db's AccountData to our AccountState.
fn account_data_to_state(data: &AccountData) -> AccountState {
    AccountState {
        nonce: data.nonce,
        balance: U256::from_big_endian(&data.balance),
        code_hash: H256::from(data.code_hash),
        storage_root: H256::from(data.storage_root),
    }
}

/// Converts our AccountState to ethrex-db's AccountData.
fn account_state_to_data(state: &AccountState) -> AccountData {
    AccountData {
        nonce: state.nonce,
        balance: u256_to_bytes(&state.balance),
        code_hash: *state.code_hash.as_fixed_bytes(),
        storage_root: *state.storage_root.as_fixed_bytes(),
    }
}

/// Converts U256 to big-endian bytes (32 bytes).
fn u256_to_bytes(value: &U256) -> [u8; 32] {
    // Handle different U256 implementations by using a portable method
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = value.byte(31 - i);
    }
    bytes
}

/// Converts StoreError from ethrex-db's DbError.
impl From<DbError> for StoreError {
    fn from(err: DbError) -> Self {
        StoreError::Custom(format!("ethrex-db error: {}", err))
    }
}

/// ethrex-db backend for state storage.
///
/// Provides optimized state storage using ethrex-db's page-based
/// storage engine with built-in trie support.
pub struct EthrexDbBackend {
    /// The underlying PagedDb
    db: Arc<RwLock<PagedDb>>,
    /// The state trie
    state_trie: Arc<RwLock<PagedStateTrie>>,
    /// Current block number
    current_block: u64,
    /// Current block hash
    current_block_hash: H256,
}

impl std::fmt::Debug for EthrexDbBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthrexDbBackend")
            .field("current_block", &self.current_block)
            .field("current_block_hash", &self.current_block_hash)
            .finish_non_exhaustive()
    }
}

impl EthrexDbBackend {
    /// Creates a new ethrex-db backend from an existing PagedDb.
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
            state_trie: Arc::new(RwLock::new(state_trie)),
            current_block: block_number,
            current_block_hash: block_hash,
        }
    }

    /// Creates a new ethrex-db backend at the given path.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let db = PagedDb::open(path)?;
        Ok(Self::new(db))
    }

    /// Creates a new ethrex-db backend at the given path with a specific initial size.
    ///
    /// The size is specified in number of 4KB pages. For example:
    /// - 16384 pages = 64MB (default)
    /// - 262144 pages = 1GB
    /// - 2097152 pages = 8GB
    pub fn open_with_size(path: &Path, initial_pages: u32) -> Result<Self, StoreError> {
        let db = PagedDb::open_with_size(path, initial_pages)?;
        Ok(Self::new(db))
    }

    /// Creates a new in-memory ethrex-db backend for testing.
    pub fn in_memory(pages: u32) -> Result<Self, StoreError> {
        let db = PagedDb::in_memory(pages)?;
        Ok(Self::new(db))
    }

    /// Returns the current block number.
    pub fn block_number(&self) -> u64 {
        self.current_block
    }

    /// Returns the current block hash.
    pub fn block_hash(&self) -> H256 {
        self.current_block_hash
    }

    /// Updates block metadata.
    pub fn set_block_info(&mut self, block_number: u64, block_hash: H256) {
        self.current_block = block_number;
        self.current_block_hash = block_hash;
    }

    /// Returns a reference to the underlying PagedDb.
    pub fn db(&self) -> Arc<RwLock<PagedDb>> {
        Arc::clone(&self.db)
    }

    /// Returns a reference to the state trie.
    pub fn state_trie(&self) -> Arc<RwLock<PagedStateTrie>> {
        Arc::clone(&self.state_trie)
    }
}

impl StateStorageBackend for EthrexDbBackend {
    // =========================================================================
    // Account Operations
    // =========================================================================

    fn get_account(&self, address_hash: &H256) -> Result<Option<AccountState>, StoreError> {
        let trie = self
            .state_trie
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        let addr_bytes: [u8; 32] = *address_hash.as_fixed_bytes();
        Ok(trie
            .get_account_by_hash(&addr_bytes)
            .map(|data| account_data_to_state(&data)))
    }

    fn get_account_by_address(&self, address: &Address) -> Result<Option<AccountState>, StoreError> {
        let address_hash = H256::from(keccak_hash(address.as_bytes()));
        self.get_account(&address_hash)
    }

    fn set_account(
        &mut self,
        address_hash: H256,
        account: AccountState,
    ) -> Result<(), StoreError> {
        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        let addr_bytes: [u8; 32] = *address_hash.as_fixed_bytes();
        let account_data = account_state_to_data(&account);
        trie.set_account_by_hash(&addr_bytes, account_data);
        Ok(())
    }

    fn delete_account(&mut self, address_hash: &H256) -> Result<(), StoreError> {
        // In ethrex-db, we set to empty account to "delete"
        // The trie will handle pruning empty accounts
        let empty_account = AccountState::empty();
        self.set_account(*address_hash, empty_account)
    }

    // =========================================================================
    // Storage Operations
    // =========================================================================

    fn get_storage(
        &self,
        address_hash: &H256,
        slot_hash: &H256,
    ) -> Result<Option<U256>, StoreError> {
        let trie = self
            .state_trie
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        let addr_bytes: [u8; 32] = *address_hash.as_fixed_bytes();
        let slot_bytes: [u8; 32] = *slot_hash.as_fixed_bytes();

        Ok(trie
            .get_storage_by_hash(&addr_bytes, &slot_bytes)
            .map(|bytes| U256::from_big_endian(&bytes)))
    }

    fn set_storage(
        &mut self,
        address_hash: H256,
        slot_hash: H256,
        value: U256,
    ) -> Result<(), StoreError> {
        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        let addr_bytes: [u8; 32] = *address_hash.as_fixed_bytes();
        let slot_bytes: [u8; 32] = *slot_hash.as_fixed_bytes();
        let value_bytes = u256_to_bytes(&value);

        let storage = trie.storage_trie_by_hash(&addr_bytes);
        storage.set_by_hash(&slot_bytes, value_bytes);
        Ok(())
    }

    // =========================================================================
    // State Root
    // =========================================================================

    fn state_root(&self) -> Result<H256, StoreError> {
        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        Ok(H256::from(trie.root_hash()))
    }

    fn compute_state_root(&mut self) -> Result<H256, StoreError> {
        self.state_root()
    }

    // =========================================================================
    // Batch Operations
    // =========================================================================

    fn apply_state_update(&mut self, update: StateUpdate) -> Result<H256, StoreError> {
        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        // Apply account changes
        for (address_hash, account_opt) in update.account_changes {
            let addr_bytes: [u8; 32] = *address_hash.as_fixed_bytes();

            match account_opt {
                Some(account) => {
                    let account_data = account_state_to_data(&account);
                    trie.set_account_by_hash(&addr_bytes, account_data);
                }
                None => {
                    // Delete = set to empty
                    trie.set_account_by_hash(&addr_bytes, AccountData::empty());
                }
            }
        }

        // Apply storage changes
        for (address_hash, slots) in update.storage_changes {
            let addr_bytes: [u8; 32] = *address_hash.as_fixed_bytes();
            let storage = trie.storage_trie_by_hash(&addr_bytes);

            for (slot_hash, value) in slots {
                let slot_bytes: [u8; 32] = *slot_hash.as_fixed_bytes();
                let value_bytes = u256_to_bytes(&value);
                storage.set_by_hash(&slot_bytes, value_bytes);
            }
        }

        Ok(H256::from(trie.root_hash()))
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        let mut db = self
            .db
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire db lock".to_string()))?;

        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        let mut batch = db.begin_batch();
        let state_root_addr = trie.save(&mut batch)?;
        batch.set_state_root(state_root_addr);
        batch.set_metadata(
            self.current_block as u32,
            self.current_block_hash.as_fixed_bytes(),
        );
        batch.commit(CommitOptions::FlushDataOnly)?;

        Ok(())
    }

    // =========================================================================
    // Proof Generation
    // =========================================================================

    fn get_account_proof(&self, _address: &Address) -> Result<Vec<Vec<u8>>, StoreError> {
        // TODO: Implement proof generation
        // ethrex-db's MerkleTrie doesn't expose proof generation yet
        // This needs to be added to ethrex-db
        Err(StoreError::Custom(
            "Proof generation not yet implemented for ethrex-db".to_string(),
        ))
    }

    fn get_storage_proofs(
        &self,
        _address: &Address,
        _slots: &[H256],
    ) -> Result<Vec<Vec<Vec<u8>>>, StoreError> {
        // TODO: Implement proof generation
        Err(StoreError::Custom(
            "Proof generation not yet implemented for ethrex-db".to_string(),
        ))
    }

    // =========================================================================
    // Iteration (for snap sync)
    // =========================================================================

    fn iter_accounts_from(
        &self,
        _start: &H256,
    ) -> Result<Box<dyn Iterator<Item = (H256, AccountState)> + '_>, StoreError> {
        // TODO: Implement iteration
        // ethrex-db's trie supports iteration but needs an adapter
        Err(StoreError::Custom(
            "Account iteration not yet implemented for ethrex-db".to_string(),
        ))
    }

    fn iter_storage_from(
        &self,
        _address_hash: &H256,
        _start: &H256,
    ) -> Result<Box<dyn Iterator<Item = (H256, U256)> + '_>, StoreError> {
        // TODO: Implement iteration
        Err(StoreError::Custom(
            "Storage iteration not yet implemented for ethrex-db".to_string(),
        ))
    }

    // =========================================================================
    // Snap Sync Support
    // =========================================================================

    fn set_accounts_batch(
        &mut self,
        accounts: Vec<(H256, AccountState)>,
    ) -> Result<(), StoreError> {
        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        let entries = accounts.into_iter().map(|(hash, state)| {
            let addr_bytes: [u8; 32] = *hash.as_fixed_bytes();
            let account_data = account_state_to_data(&state);
            (addr_bytes, account_data)
        });

        trie.set_accounts_batch(entries);
        Ok(())
    }

    fn set_storage_batch(
        &mut self,
        address_hash: H256,
        slots: Vec<(H256, U256)>,
    ) -> Result<(), StoreError> {
        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        let addr_bytes: [u8; 32] = *address_hash.as_fixed_bytes();
        let storage = trie.storage_trie_by_hash(&addr_bytes);

        let entries = slots.into_iter().map(|(slot_hash, value)| {
            let slot_bytes: [u8; 32] = *slot_hash.as_fixed_bytes();
            let value_bytes = u256_to_bytes(&value);
            (slot_bytes, value_bytes)
        });

        storage.set_batch_by_hash(entries);
        Ok(())
    }

    fn flush_storage_tries(&mut self) -> Result<usize, StoreError> {
        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        Ok(trie.flush_storage_tries())
    }

    fn persist_checkpoint(&mut self, block_number: u64, block_hash: H256) -> Result<(), StoreError> {
        self.current_block = block_number;
        self.current_block_hash = block_hash;

        let mut db = self
            .db
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire db lock".to_string()))?;

        let mut trie = self
            .state_trie
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire state trie lock".to_string()))?;

        let mut batch = db.begin_batch();
        let state_root_addr = trie.save(&mut batch)?;
        batch.set_state_root(state_root_addr);
        batch.set_metadata(block_number as u32, block_hash.as_fixed_bytes());
        batch.commit(CommitOptions::FlushDataOnly)?;

        Ok(())
    }

    fn persist_final(&mut self, block_number: u64, block_hash: H256) -> Result<(), StoreError> {
        self.persist_checkpoint(block_number, block_hash)
    }
}

// ============================================================================
// Witness Generation / Logging Wrapper
// ============================================================================

/// Represents accessed state for witness generation.
///
/// This is used by zkVM provers to capture what state was accessed during
/// block execution, enabling stateless verification.
#[derive(Clone, Debug, Default)]
pub struct StateWitness {
    /// Accounts that were read (address_hash -> AccountState)
    pub accounts_read: HashMap<H256, AccountState>,
    /// Storage slots that were read ((address_hash, slot_hash) -> value)
    pub storage_read: HashMap<(H256, H256), U256>,
    /// Accounts that were written
    pub accounts_written: HashMap<H256, Option<AccountState>>,
    /// Storage slots that were written
    pub storage_written: HashMap<(H256, H256), U256>,
}

impl StateWitness {
    /// Creates a new empty witness.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the witness for a new execution.
    pub fn clear(&mut self) {
        self.accounts_read.clear();
        self.storage_read.clear();
        self.accounts_written.clear();
        self.storage_written.clear();
    }

    /// Returns the number of unique accounts accessed.
    pub fn account_count(&self) -> usize {
        let mut keys: std::collections::HashSet<H256> = self.accounts_read.keys().copied().collect();
        keys.extend(self.accounts_written.keys().copied());
        keys.len()
    }

    /// Returns the number of unique storage slots accessed.
    pub fn storage_count(&self) -> usize {
        let mut keys: std::collections::HashSet<(H256, H256)> = self.storage_read.keys().copied().collect();
        keys.extend(self.storage_written.keys().copied());
        keys.len()
    }
}

/// Thread-safe witness handle for sharing across operations.
pub type SharedWitness = Arc<Mutex<StateWitness>>;

/// A logging wrapper around StateStorageBackend that captures state accesses.
///
/// This enables witness generation for zkVM proving by recording all state
/// reads and writes during block execution.
pub struct LoggingStateBackend<B: StateStorageBackend> {
    /// The underlying backend
    inner: B,
    /// The witness being collected
    witness: SharedWitness,
    /// Whether logging is enabled
    logging_enabled: bool,
}

impl<B: StateStorageBackend> std::fmt::Debug for LoggingStateBackend<B>
where
    B: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoggingStateBackend")
            .field("inner", &self.inner)
            .field("logging_enabled", &self.logging_enabled)
            .finish()
    }
}

impl<B: StateStorageBackend> LoggingStateBackend<B> {
    /// Creates a new logging wrapper with a fresh witness.
    pub fn new(inner: B) -> (Self, SharedWitness) {
        let witness = Arc::new(Mutex::new(StateWitness::new()));
        let wrapper = Self {
            inner,
            witness: Arc::clone(&witness),
            logging_enabled: true,
        };
        (wrapper, witness)
    }

    /// Creates a new logging wrapper with an existing witness.
    pub fn with_witness(inner: B, witness: SharedWitness) -> Self {
        Self {
            inner,
            witness,
            logging_enabled: true,
        }
    }

    /// Enables or disables logging.
    pub fn set_logging_enabled(&mut self, enabled: bool) {
        self.logging_enabled = enabled;
    }

    /// Returns a reference to the witness.
    pub fn witness(&self) -> SharedWitness {
        Arc::clone(&self.witness)
    }

    /// Returns the underlying backend.
    pub fn into_inner(self) -> B {
        self.inner
    }

    fn log_account_read(&self, address_hash: &H256, account: &AccountState) {
        if !self.logging_enabled {
            return;
        }
        if let Ok(mut witness) = self.witness.lock() {
            witness.accounts_read.insert(*address_hash, account.clone());
        }
    }

    fn log_storage_read(&self, address_hash: &H256, slot_hash: &H256, value: &U256) {
        if !self.logging_enabled {
            return;
        }
        if let Ok(mut witness) = self.witness.lock() {
            witness.storage_read.insert((*address_hash, *slot_hash), *value);
        }
    }

    fn log_account_write(&self, address_hash: &H256, account: Option<&AccountState>) {
        if !self.logging_enabled {
            return;
        }
        if let Ok(mut witness) = self.witness.lock() {
            witness.accounts_written.insert(*address_hash, account.cloned());
        }
    }

    fn log_storage_write(&self, address_hash: &H256, slot_hash: &H256, value: &U256) {
        if !self.logging_enabled {
            return;
        }
        if let Ok(mut witness) = self.witness.lock() {
            witness.storage_written.insert((*address_hash, *slot_hash), *value);
        }
    }
}

impl<B: StateStorageBackend> StateStorageBackend for LoggingStateBackend<B> {
    fn get_account(&self, address_hash: &H256) -> Result<Option<AccountState>, StoreError> {
        let result = self.inner.get_account(address_hash)?;
        if let Some(ref account) = result {
            self.log_account_read(address_hash, account);
        }
        Ok(result)
    }

    fn get_account_by_address(&self, address: &Address) -> Result<Option<AccountState>, StoreError> {
        let result = self.inner.get_account_by_address(address)?;
        if let Some(ref account) = result {
            let address_hash = H256::from(keccak_hash(address.as_bytes()));
            self.log_account_read(&address_hash, account);
        }
        Ok(result)
    }

    fn set_account(&mut self, address_hash: H256, account: AccountState) -> Result<(), StoreError> {
        self.log_account_write(&address_hash, Some(&account));
        self.inner.set_account(address_hash, account)
    }

    fn delete_account(&mut self, address_hash: &H256) -> Result<(), StoreError> {
        self.log_account_write(address_hash, None);
        self.inner.delete_account(address_hash)
    }

    fn get_storage(&self, address_hash: &H256, slot_hash: &H256) -> Result<Option<U256>, StoreError> {
        let result = self.inner.get_storage(address_hash, slot_hash)?;
        if let Some(ref value) = result {
            self.log_storage_read(address_hash, slot_hash, value);
        }
        Ok(result)
    }

    fn set_storage(&mut self, address_hash: H256, slot_hash: H256, value: U256) -> Result<(), StoreError> {
        self.log_storage_write(&address_hash, &slot_hash, &value);
        self.inner.set_storage(address_hash, slot_hash, value)
    }

    fn state_root(&self) -> Result<H256, StoreError> {
        self.inner.state_root()
    }

    fn compute_state_root(&mut self) -> Result<H256, StoreError> {
        self.inner.compute_state_root()
    }

    fn apply_state_update(&mut self, update: StateUpdate) -> Result<H256, StoreError> {
        // Log all the updates
        for (address_hash, account_opt) in &update.account_changes {
            self.log_account_write(address_hash, account_opt.as_ref());
        }
        for (address_hash, slots) in &update.storage_changes {
            for (slot_hash, value) in slots {
                self.log_storage_write(address_hash, slot_hash, value);
            }
        }
        self.inner.apply_state_update(update)
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        self.inner.commit()
    }

    fn get_account_proof(&self, address: &Address) -> Result<Vec<Vec<u8>>, StoreError> {
        self.inner.get_account_proof(address)
    }

    fn get_storage_proofs(&self, address: &Address, slots: &[H256]) -> Result<Vec<Vec<Vec<u8>>>, StoreError> {
        self.inner.get_storage_proofs(address, slots)
    }

    fn iter_accounts_from(&self, start: &H256) -> Result<Box<dyn Iterator<Item = (H256, AccountState)> + '_>, StoreError> {
        self.inner.iter_accounts_from(start)
    }

    fn iter_storage_from(&self, address_hash: &H256, start: &H256) -> Result<Box<dyn Iterator<Item = (H256, U256)> + '_>, StoreError> {
        self.inner.iter_storage_from(address_hash, start)
    }

    fn set_accounts_batch(&mut self, accounts: Vec<(H256, AccountState)>) -> Result<(), StoreError> {
        for (address_hash, account) in &accounts {
            self.log_account_write(address_hash, Some(account));
        }
        self.inner.set_accounts_batch(accounts)
    }

    fn set_storage_batch(&mut self, address_hash: H256, slots: Vec<(H256, U256)>) -> Result<(), StoreError> {
        for (slot_hash, value) in &slots {
            self.log_storage_write(&address_hash, slot_hash, value);
        }
        self.inner.set_storage_batch(address_hash, slots)
    }

    fn flush_storage_tries(&mut self) -> Result<usize, StoreError> {
        self.inner.flush_storage_tries()
    }

    fn persist_checkpoint(&mut self, block_number: u64, block_hash: H256) -> Result<(), StoreError> {
        self.inner.persist_checkpoint(block_number, block_hash)
    }

    fn persist_final(&mut self, block_number: u64, block_hash: H256) -> Result<(), StoreError> {
        self.inner.persist_final(block_number, block_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_roundtrip() {
        let mut backend = EthrexDbBackend::in_memory(1000).unwrap();

        let address_hash = H256::from([1u8; 32]);
        let account = AccountState {
            nonce: 42,
            balance: U256::from(1000),
            code_hash: AccountState::EMPTY_CODE_HASH,
            storage_root: AccountState::EMPTY_STORAGE_ROOT,
        };

        backend.set_account(address_hash, account.clone()).unwrap();
        let retrieved = backend.get_account(&address_hash).unwrap().unwrap();

        assert_eq!(retrieved.nonce, 42);
        assert_eq!(retrieved.balance, U256::from(1000));
    }

    #[test]
    fn test_storage_roundtrip() {
        let mut backend = EthrexDbBackend::in_memory(1000).unwrap();

        let address_hash = H256::from([1u8; 32]);
        let slot_hash = H256::from([2u8; 32]);
        let value = U256::from(12345);

        backend
            .set_storage(address_hash, slot_hash, value)
            .unwrap();
        let retrieved = backend.get_storage(&address_hash, &slot_hash).unwrap().unwrap();

        assert_eq!(retrieved, value);
    }

    #[test]
    fn test_state_root_changes() {
        let mut backend = EthrexDbBackend::in_memory(1000).unwrap();

        let empty_root = backend.state_root().unwrap();

        let address_hash = H256::from([1u8; 32]);
        let account = AccountState {
            nonce: 1,
            balance: U256::from(100),
            code_hash: AccountState::EMPTY_CODE_HASH,
            storage_root: AccountState::EMPTY_STORAGE_ROOT,
        };

        backend.set_account(address_hash, account).unwrap();
        let new_root = backend.state_root().unwrap();

        assert_ne!(empty_root, new_root);
    }

    #[test]
    fn test_batch_update() {
        let mut backend = EthrexDbBackend::in_memory(1000).unwrap();

        let mut update = StateUpdate::new();

        let addr1 = H256::from([1u8; 32]);
        let addr2 = H256::from([2u8; 32]);

        update.set_account(
            addr1,
            AccountState {
                nonce: 1,
                balance: U256::from(100),
                code_hash: AccountState::EMPTY_CODE_HASH,
                storage_root: AccountState::EMPTY_STORAGE_ROOT,
            },
        );
        update.set_account(
            addr2,
            AccountState {
                nonce: 2,
                balance: U256::from(200),
                code_hash: AccountState::EMPTY_CODE_HASH,
                storage_root: AccountState::EMPTY_STORAGE_ROOT,
            },
        );

        let _new_root = backend.apply_state_update(update).unwrap();

        assert_eq!(backend.get_account(&addr1).unwrap().unwrap().nonce, 1);
        assert_eq!(backend.get_account(&addr2).unwrap().unwrap().nonce, 2);
    }

    #[test]
    fn test_snap_sync_batch() {
        let mut backend = EthrexDbBackend::in_memory(1000).unwrap();

        let accounts = vec![
            (
                H256::from([1u8; 32]),
                AccountState {
                    nonce: 1,
                    balance: U256::from(100),
                    code_hash: AccountState::EMPTY_CODE_HASH,
                    storage_root: AccountState::EMPTY_STORAGE_ROOT,
                },
            ),
            (
                H256::from([2u8; 32]),
                AccountState {
                    nonce: 2,
                    balance: U256::from(200),
                    code_hash: AccountState::EMPTY_CODE_HASH,
                    storage_root: AccountState::EMPTY_STORAGE_ROOT,
                },
            ),
        ];

        backend.set_accounts_batch(accounts).unwrap();

        assert_eq!(
            backend
                .get_account(&H256::from([1u8; 32]))
                .unwrap()
                .unwrap()
                .nonce,
            1
        );
        assert_eq!(
            backend
                .get_account(&H256::from([2u8; 32]))
                .unwrap()
                .unwrap()
                .nonce,
            2
        );
    }

    #[test]
    fn test_witness_logging() {
        let backend = EthrexDbBackend::in_memory(1000).unwrap();
        let (mut logging_backend, witness) = LoggingStateBackend::new(backend);

        let address_hash = H256::from([1u8; 32]);
        let account = AccountState {
            nonce: 42,
            balance: U256::from(1000),
            code_hash: AccountState::EMPTY_CODE_HASH,
            storage_root: AccountState::EMPTY_STORAGE_ROOT,
        };

        // Write should be logged
        logging_backend.set_account(address_hash, account.clone()).unwrap();

        {
            let w = witness.lock().unwrap();
            assert!(w.accounts_written.contains_key(&address_hash));
            assert_eq!(w.accounts_written.len(), 1);
        }

        // Read should be logged
        let _ = logging_backend.get_account(&address_hash).unwrap();

        {
            let w = witness.lock().unwrap();
            assert!(w.accounts_read.contains_key(&address_hash));
            assert_eq!(w.accounts_read.len(), 1);
        }
    }

    #[test]
    fn test_witness_storage_logging() {
        let backend = EthrexDbBackend::in_memory(1000).unwrap();
        let (mut logging_backend, witness) = LoggingStateBackend::new(backend);

        let address_hash = H256::from([1u8; 32]);
        let slot_hash = H256::from([2u8; 32]);
        let value = U256::from(12345);

        // Write storage
        logging_backend.set_storage(address_hash, slot_hash, value).unwrap();

        {
            let w = witness.lock().unwrap();
            assert!(w.storage_written.contains_key(&(address_hash, slot_hash)));
            assert_eq!(w.storage_count(), 1);
        }

        // Read storage
        let _ = logging_backend.get_storage(&address_hash, &slot_hash).unwrap();

        {
            let w = witness.lock().unwrap();
            assert!(w.storage_read.contains_key(&(address_hash, slot_hash)));
        }
    }

    #[test]
    fn test_witness_clear() {
        let backend = EthrexDbBackend::in_memory(1000).unwrap();
        let (mut logging_backend, witness) = LoggingStateBackend::new(backend);

        let address_hash = H256::from([1u8; 32]);
        let account = AccountState {
            nonce: 1,
            balance: U256::from(100),
            code_hash: AccountState::EMPTY_CODE_HASH,
            storage_root: AccountState::EMPTY_STORAGE_ROOT,
        };

        logging_backend.set_account(address_hash, account).unwrap();

        // Clear the witness
        {
            let mut w = witness.lock().unwrap();
            assert_eq!(w.account_count(), 1);
            w.clear();
            assert_eq!(w.account_count(), 0);
        }
    }

    #[test]
    fn test_logging_can_be_disabled() {
        let backend = EthrexDbBackend::in_memory(1000).unwrap();
        let (mut logging_backend, witness) = LoggingStateBackend::new(backend);

        // Disable logging
        logging_backend.set_logging_enabled(false);

        let address_hash = H256::from([1u8; 32]);
        let account = AccountState {
            nonce: 1,
            balance: U256::from(100),
            code_hash: AccountState::EMPTY_CODE_HASH,
            storage_root: AccountState::EMPTY_STORAGE_ROOT,
        };

        logging_backend.set_account(address_hash, account).unwrap();

        // Nothing should be logged
        {
            let w = witness.lock().unwrap();
            assert_eq!(w.account_count(), 0);
        }
    }
}
