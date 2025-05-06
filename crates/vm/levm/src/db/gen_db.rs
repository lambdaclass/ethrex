use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::types::Account;
use ethrex_common::types::Fork;
use ethrex_common::Address;
use ethrex_common::U256;
use keccak_hash::H256;

use crate::errors::InternalError;
use crate::errors::VMError;
use crate::vm::Substate;
use crate::vm::VM;

use super::cache;
use super::error::DatabaseError;
use super::CacheDB;
use super::Database;

#[derive(Clone)]
pub struct GeneralizedDatabase {
    pub store: Arc<dyn Database>,
    pub cache: CacheDB,
}

impl GeneralizedDatabase {
    pub fn new(store: Arc<dyn Database>, cache: CacheDB) -> Self {
        Self { store, cache }
    }

    // ================== Account related functions =====================
    /// Gets account, first checking the cache and then the database
    /// (caching in the second case)
    pub fn get_account(&mut self, address: Address) -> Result<&Account, DatabaseError> {
        if !cache::account_is_cached(&self.cache, &address) {
            let account = self.store.get_account(address)?.clone();
            cache::insert_account(&mut self.cache, address, account);
        }
        cache::get_account(&self.cache, &address)
            .ok_or(DatabaseError::Custom("Cache error".to_owned()))
    }

    /// **Accesses to an account's information.**
    ///
    /// Accessed accounts are stored in the `touched_accounts` set.
    /// Accessed accounts take place in some gas cost computation.
    pub fn access_account(
        &mut self,
        accrued_substate: &mut Substate,
        address: Address,
    ) -> Result<(&Account, bool), DatabaseError> {
        let address_was_cold = accrued_substate.touched_accounts.insert(address);
        let account = self.get_account(address)?;

        Ok((account, address_was_cold))
    }
}

impl<'a> VM<'a> {
    // ================== Account related functions =====================

    pub fn get_account_mut(&mut self, address: Address) -> Result<&mut Account, VMError> {
        let backup_account = self.db.get_account(address)?.clone();

        if let Ok(frame) = self.current_call_frame_mut() {
            frame
                .cache_backup
                .entry(address)
                .or_insert_with(|| Some(backup_account));
        }

        let account = cache::get_account_mut(&mut self.db.cache, &address)
            .ok_or(VMError::Internal(InternalError::AccountNotFound))?;

        Ok(account)
    }

    pub fn increase_account_balance(
        &mut self,
        address: Address,
        increase: U256,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.info.balance = account
            .info
            .balance
            .checked_add(increase)
            .ok_or(VMError::BalanceOverflow)?;
        Ok(())
    }

    pub fn decrease_account_balance(
        &mut self,
        address: Address,
        decrease: U256,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.info.balance = account
            .info
            .balance
            .checked_sub(decrease)
            .ok_or(VMError::BalanceUnderflow)?;
        Ok(())
    }

    /// Updates bytecode of given account.
    pub fn update_account_bytecode(
        &mut self,
        address: Address,
        new_bytecode: Bytes,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.set_code(new_bytecode);
        Ok(())
    }

    // =================== Nonce related functions ======================
    pub fn increment_account_nonce(&mut self, address: Address) -> Result<u64, VMError> {
        let account = self.get_account_mut(address)?;
        account.info.nonce = account
            .info
            .nonce
            .checked_add(1)
            .ok_or(VMError::NonceOverflow)?;
        Ok(account.info.nonce)
    }

    /// Inserts account to cache backing up the previus state of it in the CacheBackup (if it wasn't already backed up)
    pub fn insert_account(&mut self, address: Address, account: Account) -> Result<(), VMError> {
        let previous_account = cache::insert_account(&mut self.db.cache, address, account);

        if let Ok(frame) = self.current_call_frame_mut() {
            frame
                .cache_backup
                .entry(address)
                .or_insert_with(|| previous_account.as_ref().map(|account| (*account).clone()));
        }

        Ok(())
    }

    /// Removes account from cache backing up the previus state of it in the CacheBackup (if it wasn't already backed up)
    pub fn remove_account(&mut self, address: Address) -> Result<(), VMError> {
        let previous_account = cache::remove_account(&mut self.db.cache, &address);

        if let Ok(frame) = self.current_call_frame_mut() {
            frame
                .cache_backup
                .entry(address)
                .or_insert_with(|| previous_account.as_ref().map(|account| (*account).clone()));
        }

        Ok(())
    }

    /// Gets original storage value of an account, caching it if not already cached.
    /// Also saves the original value for future gas calculations.
    pub fn get_original_storage(&mut self, address: Address, key: H256) -> Result<U256, VMError> {
        if let Some(value) = self
            .storage_original_values
            .get(&address)
            .and_then(|account_storage| account_storage.get(&key))
        {
            return Ok(*value);
        }

        let value = self.get_storage_value(address, key)?;
        self.storage_original_values
            .entry(address)
            .or_default()
            .insert(key, value);
        Ok(value)
    }

    /// Accesses to an account's storage slot and returns the value in it.
    ///
    /// Accessed storage slots are stored in the `touched_storage_slots` set.
    /// Accessed storage slots take place in some gas cost computation.
    pub fn access_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<(U256, bool), VMError> {
        // [EIP-2929] - Introduced conditional tracking of accessed storage slots for Berlin and later specs.
        let mut storage_slot_was_cold = false;
        if self.env.config.fork >= Fork::Berlin {
            storage_slot_was_cold = self
                .accrued_substate
                .touched_storage_slots
                .entry(address)
                .or_default()
                .insert(key);
        }

        let storage_slot = self.get_storage_value(address, key)?;

        Ok((storage_slot, storage_slot_was_cold))
    }

    /// Gets storage value of an account, caching it if not already cached.
    pub fn get_storage_value(&mut self, address: Address, key: H256) -> Result<U256, VMError> {
        if let Some(account) = cache::get_account(&self.db.cache, &address) {
            if let Some(value) = account.storage.get(&key) {
                return Ok(*value);
            }
        }

        let value = self.db.store.get_storage_value(address, key)?;

        // When getting storage value of an account that's not yet cached we need to store it in the account
        // We don't actually know if the account is cached so we cache it anyway
        let account = self.get_account_mut(address)?;
        account.storage.entry(key).or_insert(value);

        Ok(value)
    }

    /// Updates storage of an account, caching it if not already cached.
    pub fn update_account_storage(
        &mut self,
        address: Address,
        key: H256,
        new_value: U256,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.storage.insert(key, new_value);
        Ok(())
    }
}
