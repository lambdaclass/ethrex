//! State Engine for ethrex_db storage backend.
//!
//! Provides high-performance state storage using ethrex_db's
//! memory-mapped database with Copy-on-Write semantics.

#[cfg(feature = "ethrex_db")]
use std::collections::HashMap;
#[cfg(feature = "ethrex_db")]
use std::path::Path;
#[cfg(feature = "ethrex_db")]
use std::sync::{Arc, RwLock};

#[cfg(feature = "ethrex_db")]
use ethrex_common::{
    Address, H256, U256,
    types::{AccountInfo, AccountState, Code},
};
#[cfg(feature = "ethrex_db")]
use ethrex_crypto::keccak::keccak_hash;

#[cfg(feature = "ethrex_db")]
use crate::error::StoreError;

#[cfg(feature = "ethrex_db")]
use ethrex_db::{
    Account as EthrexDbAccount, Block as EthrexDbBlock, Blockchain as EthrexDbBlockchain, PagedDb,
    ReadOnlyWorldState, WorldState,
};

/// Result type for StateEngine operations.
#[cfg(feature = "ethrex_db")]
pub type StateEngineResult<T> = Result<T, StoreError>;

/// High-performance state storage using ethrex_db.
///
/// Uses memory-mapped files with Copy-on-Write semantics for efficient
/// state management during block execution.
#[cfg(feature = "ethrex_db")]
pub struct EthrexDbStateEngine {
    /// The underlying blockchain manager.
    blockchain: Arc<RwLock<EthrexDbBlockchain>>,
}

#[cfg(feature = "ethrex_db")]
impl std::fmt::Debug for EthrexDbStateEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthrexDbStateEngine")
            .field("blockchain", &"<Blockchain>")
            .finish()
    }
}

#[cfg(feature = "ethrex_db")]
impl EthrexDbStateEngine {
    /// Creates a new EthrexDbStateEngine with a database at the given path.
    pub fn new(db_path: &Path) -> Result<Self, StoreError> {
        let paged_db = PagedDb::open(db_path)
            .map_err(|e| StoreError::Custom(format!("Failed to open PagedDb: {:?}", e)))?;
        let blockchain = EthrexDbBlockchain::new(paged_db);

        Ok(Self {
            blockchain: Arc::new(RwLock::new(blockchain)),
        })
    }

    /// Creates a new EthrexDbStateEngine with an in-memory database.
    pub fn in_memory() -> Result<Self, StoreError> {
        let paged_db = PagedDb::in_memory(10000).map_err(|e| {
            StoreError::Custom(format!("Failed to create in-memory PagedDb: {:?}", e))
        })?;
        let blockchain = EthrexDbBlockchain::new(paged_db);

        Ok(Self {
            blockchain: Arc::new(RwLock::new(blockchain)),
        })
    }

    /// Returns a reference to the underlying blockchain.
    pub fn blockchain(&self) -> &Arc<RwLock<EthrexDbBlockchain>> {
        &self.blockchain
    }

    fn hash_address(address: &Address) -> [u8; 32] {
        keccak_hash(address.as_bytes())
    }

    fn account_to_state(account: &EthrexDbAccount) -> AccountState {
        AccountState {
            nonce: account.nonce,
            balance: account.balance,
            storage_root: account.storage_root,
            code_hash: account.code_hash,
        }
    }

    fn account_to_info(account: &EthrexDbAccount) -> AccountInfo {
        AccountInfo {
            code_hash: account.code_hash,
            balance: account.balance,
            nonce: account.nonce,
        }
    }

    // ========================================================================
    // Account Operations
    // ========================================================================

    /// Gets account info (balance, nonce, code_hash) at a specific block.
    pub fn get_account_info(
        &self,
        block_hash: H256,
        address: Address,
    ) -> StateEngineResult<Option<AccountInfo>> {
        let blockchain = self.blockchain.read().map_err(|_| StoreError::LockError)?;

        let addr_hash = Self::hash_address(&address);
        let addr_h256 = H256::from(addr_hash);

        if let Some(account) = blockchain.get_account(&block_hash, &addr_h256) {
            return Ok(Some(Self::account_to_info(&account)));
        }

        if let Some(account) = blockchain.get_finalized_account_by_hash(&addr_hash) {
            return Ok(Some(Self::account_to_info(&account)));
        }

        Ok(None)
    }

    /// Gets full account state (including storage_root) at a specific block.
    pub fn get_account_state(
        &self,
        block_hash: H256,
        address: Address,
    ) -> StateEngineResult<Option<AccountState>> {
        let blockchain = self.blockchain.read().map_err(|_| StoreError::LockError)?;

        let addr_hash = Self::hash_address(&address);
        let addr_h256 = H256::from(addr_hash);

        if let Some(account) = blockchain.get_account(&block_hash, &addr_h256) {
            return Ok(Some(Self::account_to_state(&account)));
        }

        if let Some(account) = blockchain.get_finalized_account_by_hash(&addr_hash) {
            return Ok(Some(Self::account_to_state(&account)));
        }

        Ok(None)
    }

    // ========================================================================
    // Storage Operations
    // ========================================================================

    /// Gets a storage value for an account at a specific block.
    pub fn get_storage(
        &self,
        block_hash: H256,
        address: Address,
        key: H256,
    ) -> StateEngineResult<U256> {
        let blockchain = self.blockchain.read().map_err(|_| StoreError::LockError)?;

        let addr_hash = Self::hash_address(&address);
        let addr_h256 = H256::from(addr_hash);
        let key_hash = keccak_hash(key.as_bytes());
        let key_h256 = H256::from(key_hash);

        if let Some(value) = blockchain.get_storage(&block_hash, &addr_h256, &key_h256) {
            return Ok(value);
        }

        if let Some(value) = blockchain.get_finalized_storage_by_hash(&addr_hash, &key_hash) {
            return Ok(value);
        }

        Ok(U256::zero())
    }

    // ========================================================================
    // Code Operations
    // ========================================================================

    /// Gets contract bytecode by code hash.
    pub fn get_code(&self, code_hash: H256) -> StateEngineResult<Option<Code>> {
        if code_hash == *ethrex_common::constants::EMPTY_KECCACK_HASH {
            return Ok(Some(Code::default()));
        }

        let blockchain = self.blockchain.read().map_err(|_| StoreError::LockError)?;

        if let Some(bytes) = blockchain.get_code(code_hash.as_fixed_bytes()) {
            let code = Code::from_bytecode_unchecked(bytes.into(), code_hash);
            return Ok(Some(code));
        }

        Ok(None)
    }

    /// Stores contract bytecode.
    pub fn store_code(&self, code: Code) -> StateEngineResult<()> {
        if code.hash == *ethrex_common::constants::EMPTY_KECCACK_HASH {
            return Ok(());
        }

        let blockchain = self.blockchain.read().map_err(|_| StoreError::LockError)?;

        blockchain.store_code(*code.hash.as_fixed_bytes(), code.bytecode.to_vec());
        Ok(())
    }

    // ========================================================================
    // State Root
    // ========================================================================

    /// Gets the state root.
    pub fn state_root(&self) -> StateEngineResult<H256> {
        let blockchain = self.blockchain.read().map_err(|_| StoreError::LockError)?;

        Ok(H256::from(blockchain.state_root()))
    }

    // ========================================================================
    // Block Lifecycle
    // ========================================================================

    /// Starts a new block execution context.
    pub fn start_block(
        &self,
        parent_hash: H256,
        block_hash: H256,
        block_number: u64,
    ) -> StateEngineResult<EthrexDbBlockBuilder> {
        let blockchain = self.blockchain.write().map_err(|_| StoreError::LockError)?;

        let block = blockchain
            .start_new(parent_hash, block_hash, block_number)
            .map_err(|e| StoreError::Custom(format!("Failed to start block: {:?}", e)))?;

        Ok(EthrexDbBlockBuilder::new(
            parent_hash,
            block_hash,
            block_number,
            block,
            self.blockchain.clone(),
        ))
    }

    /// Finalizes blocks up to the given hash.
    pub fn finalize_block(&self, block_hash: H256) -> StateEngineResult<()> {
        let blockchain = self.blockchain.write().map_err(|_| StoreError::LockError)?;

        blockchain
            .finalize(block_hash)
            .map_err(|e| StoreError::Custom(format!("Failed to finalize block: {:?}", e)))?;

        Ok(())
    }

    /// Handles a fork choice update.
    pub fn fork_choice_update(
        &self,
        head_hash: H256,
        safe_hash: Option<H256>,
        finalized_hash: Option<H256>,
    ) -> StateEngineResult<()> {
        let blockchain = self.blockchain.write().map_err(|_| StoreError::LockError)?;

        blockchain
            .fork_choice_update(head_hash, safe_hash, finalized_hash)
            .map_err(|e| StoreError::Custom(format!("Fork choice update failed: {:?}", e)))?;

        Ok(())
    }
}

/// Block builder for accumulating state changes during block execution.
#[cfg(feature = "ethrex_db")]
pub struct EthrexDbBlockBuilder {
    parent_hash: H256,
    block_hash: H256,
    block_number: u64,
    block: Option<EthrexDbBlock>,
    blockchain: Arc<RwLock<EthrexDbBlockchain>>,
    code_changes: HashMap<H256, Code>,
}

#[cfg(feature = "ethrex_db")]
impl std::fmt::Debug for EthrexDbBlockBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthrexDbBlockBuilder")
            .field("parent_hash", &self.parent_hash)
            .field("block_hash", &self.block_hash)
            .field("block_number", &self.block_number)
            .field("has_block", &self.block.is_some())
            .field("code_changes_count", &self.code_changes.len())
            .finish()
    }
}

#[cfg(feature = "ethrex_db")]
impl EthrexDbBlockBuilder {
    fn new(
        parent_hash: H256,
        block_hash: H256,
        block_number: u64,
        block: EthrexDbBlock,
        blockchain: Arc<RwLock<EthrexDbBlockchain>>,
    ) -> Self {
        Self {
            parent_hash,
            block_hash,
            block_number,
            block: Some(block),
            blockchain,
            code_changes: HashMap::new(),
        }
    }

    fn hash_address(address: &Address) -> H256 {
        H256::from(keccak_hash(address.as_bytes()))
    }

    fn hash_key(key: &H256) -> H256 {
        H256::from(keccak_hash(key.as_bytes()))
    }

    pub fn block_hash(&self) -> H256 {
        self.block_hash
    }

    pub fn parent_hash(&self) -> H256 {
        self.parent_hash
    }

    pub fn block_number(&self) -> u64 {
        self.block_number
    }

    pub fn set_account(&mut self, address: Address, account: AccountState) {
        if let Some(block) = &mut self.block {
            let addr_hash = Self::hash_address(&address);
            let db_account = EthrexDbAccount {
                nonce: account.nonce,
                balance: account.balance,
                code_hash: account.code_hash,
                storage_root: account.storage_root,
            };
            block.set_account(addr_hash, db_account);
        }
    }

    pub fn set_storage(&mut self, address: Address, key: H256, value: U256) {
        if let Some(block) = &mut self.block {
            let addr_hash = Self::hash_address(&address);
            let key_hash = Self::hash_key(&key);
            block.set_storage(addr_hash, key_hash, value);
        }
    }

    pub fn increment_nonce(&mut self, address: Address) {
        if let Some(block) = &mut self.block {
            let addr_hash = Self::hash_address(&address);
            block.increment_nonce(&addr_hash);
        }
    }

    pub fn add_balance(&mut self, address: Address, amount: U256) {
        if let Some(block) = &mut self.block {
            let addr_hash = Self::hash_address(&address);
            block.add_balance(&addr_hash, amount);
        }
    }

    pub fn sub_balance(&mut self, address: Address, amount: U256) -> bool {
        if let Some(block) = &mut self.block {
            let addr_hash = Self::hash_address(&address);
            return block.sub_balance(&addr_hash, amount);
        }
        false
    }

    pub fn delete_account(&mut self, address: Address) {
        if let Some(block) = &mut self.block {
            let addr_hash = Self::hash_address(&address);
            block.delete_account(&addr_hash);
        }
    }

    pub fn store_code(&mut self, code: Code) {
        self.code_changes.insert(code.hash, code.clone());

        if let Ok(blockchain) = self.blockchain.read() {
            blockchain.store_code(*code.hash.as_fixed_bytes(), code.bytecode.to_vec());
        }
    }

    pub fn state_root(&mut self) -> H256 {
        *ethrex_trie::EMPTY_TRIE_HASH
    }

    pub fn get_account(&self, address: Address) -> Option<AccountState> {
        let addr_hash = Self::hash_address(&address);

        if let Some(block) = &self.block {
            if let Some(account) = block.get_account(&addr_hash) {
                return Some(AccountState {
                    nonce: account.nonce,
                    balance: account.balance,
                    storage_root: account.storage_root,
                    code_hash: account.code_hash,
                });
            }
        }

        if let Ok(blockchain) = self.blockchain.read() {
            if let Some(account) =
                blockchain.get_finalized_account_by_hash(addr_hash.as_fixed_bytes())
            {
                return Some(AccountState {
                    nonce: account.nonce,
                    balance: account.balance,
                    storage_root: account.storage_root,
                    code_hash: account.code_hash,
                });
            }
        }

        None
    }

    pub fn get_storage(&self, address: Address, key: H256) -> U256 {
        let addr_hash = Self::hash_address(&address);
        let key_hash = Self::hash_key(&key);

        if let Some(block) = &self.block {
            if let Some(value) = block.get_storage(&addr_hash, &key_hash) {
                return value;
            }
        }

        if let Ok(blockchain) = self.blockchain.read() {
            if let Some(value) = blockchain.get_finalized_storage_by_hash(
                addr_hash.as_fixed_bytes(),
                key_hash.as_fixed_bytes(),
            ) {
                return value;
            }
        }

        U256::zero()
    }
}

#[cfg(all(test, feature = "ethrex_db"))]
mod tests {
    use super::*;

    #[test]
    fn test_state_engine_creation() {
        let engine = EthrexDbStateEngine::in_memory();
        assert!(engine.is_ok());
    }

    #[test]
    fn test_state_engine_basic_ops() {
        let engine = EthrexDbStateEngine::in_memory().expect("Failed to create engine");

        let block_hash = H256::repeat_byte(1);
        let address = Address::repeat_byte(0x42);

        let result = engine.get_account_info(block_hash, address);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());

        let result = engine.get_storage(block_hash, address, H256::zero());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), U256::zero());
    }
}
