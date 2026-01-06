//! Storage adapter bridging ethrex's Database to pevm's Storage trait
//!
//! pevm's Storage trait requires:
//! - basic(address) -> Option<AccountBasic>
//! - code_hash(address) -> Option<B256>
//! - code_by_hash(code_hash) -> Option<EvmCode>
//! - has_storage(address) -> bool
//! - storage(address, index) -> U256
//! - block_hash(number) -> B256

use ethrex_levm::db::Database;
use ethrex_levm::errors::DatabaseError;

use super::types::{
    alloy_b256_to_ethrex, ethrex_h256_to_alloy,
    ethrex_u256_to_alloy,
};

use alloy_primitives::{Address as AlloyAddress, B256, U256 as AlloyU256};
use ethrex_common::constants::EMPTY_TRIE_HASH;
use pevm::{AccountBasic, EvmCode, Storage};
use std::fmt;
use std::sync::Arc;

/// Error type for storage adapter operations
#[derive(Debug)]
pub struct PevmStorageError(pub String);

impl fmt::Display for PevmStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PEVM Storage Error: {}", self.0)
    }
}

impl std::error::Error for PevmStorageError {}

impl From<DatabaseError> for PevmStorageError {
    fn from(err: DatabaseError) -> Self {
        PevmStorageError(format!("{:?}", err))
    }
}

/// Adapter that wraps an ethrex Database to implement pevm's Storage trait
pub struct PevmStorageAdapter {
    db: Arc<dyn Database>,
}

impl PevmStorageAdapter {
    /// Create a new storage adapter wrapping the given Database
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }
}

impl Storage for PevmStorageAdapter {
    type Error = PevmStorageError;

    /// Get basic account information (nonce and balance)
    fn basic(&self, address: &AlloyAddress) -> Result<Option<AccountBasic>, Self::Error> {
        let ethrex_addr = super::types::alloy_addr_to_ethrex(address);

        let account_state = self.db.get_account_state(ethrex_addr)?;
        // If account doesn't exist, it will have default values (nonce=0, balance=0)
        // We can check if it's essentially empty
        if account_state.nonce == 0
            && account_state.balance == ethrex_common::U256::zero()
            && account_state.code_hash == *ethrex_common::constants::EMPTY_KECCACK_HASH
        {
            return Ok(None);
        }

        Ok(Some(AccountBasic {
            nonce: account_state.nonce,
            balance: ethrex_u256_to_alloy(&account_state.balance),
        }))
    }

    /// Get the code hash for an account
    fn code_hash(&self, address: &AlloyAddress) -> Result<Option<B256>, Self::Error> {
        let ethrex_addr = super::types::alloy_addr_to_ethrex(address);

        let account_state = self.db.get_account_state(ethrex_addr)?;
        if account_state.code_hash == *ethrex_common::constants::EMPTY_KECCACK_HASH {
            return Ok(None);
        }

        Ok(Some(ethrex_h256_to_alloy(&account_state.code_hash)))
    }

    /// Get bytecode by its hash
    fn code_by_hash(&self, code_hash: &B256) -> Result<Option<EvmCode>, Self::Error> {
        let ethrex_hash = alloy_b256_to_ethrex(code_hash);

        // Empty code hash returns None (no code)
        if ethrex_hash == *ethrex_common::constants::EMPTY_KECCACK_HASH {
            return Ok(None);
        }

        match self.db.get_account_code(ethrex_hash) {
            Ok(code) => {
                // Convert ethrex bytecode to revm Bytecode, then to pevm EvmCode
                let revm_bytecode = revm::primitives::Bytecode::new_raw(
                    alloy_primitives::Bytes::copy_from_slice(&code.bytecode),
                );
                Ok(Some(EvmCode::from(revm_bytecode)))
            }
            Err(_) => Ok(None), // Code not found
        }
    }

    /// Check if an account has storage (non-empty storage root)
    /// This is used for EIP-7610 support
    fn has_storage(&self, address: &AlloyAddress) -> Result<bool, Self::Error> {
        let ethrex_addr = super::types::alloy_addr_to_ethrex(address);

        let account_state = self.db.get_account_state(ethrex_addr)?;
        Ok(account_state.storage_root != *EMPTY_TRIE_HASH)
    }

    /// Get a storage value at the given index
    fn storage(&self, address: &AlloyAddress, index: &AlloyU256) -> Result<AlloyU256, Self::Error> {
        let ethrex_addr = super::types::alloy_addr_to_ethrex(address);
        let ethrex_key = ethrex_common::H256::from_slice(&index.to_be_bytes::<32>());

        let value = self.db.get_storage_value(ethrex_addr, ethrex_key)?;
        Ok(ethrex_u256_to_alloy(&value))
    }

    /// Get block hash by block number
    fn block_hash(&self, number: &u64) -> Result<B256, Self::Error> {
        let hash = self.db.get_block_hash(*number)?;
        Ok(ethrex_h256_to_alloy(&hash))
    }
}

#[cfg(test)]
mod tests {
    // TODO: Add tests with mock Database
}
