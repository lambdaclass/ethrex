use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::types::Account;
use ethrex_common::Address;
use ethrex_common::U256;
use keccak_hash::H256;

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

    /*
        Each callframe has a CallFrameBackup, which contains:

        - A list with account infos of every account that was modified so far (balance, nonce, bytecode/code hash)
        - A list with a tuple (address, storage) that contains, for every account whose storage was accessed, a hashmap
        of the storage slots that were modified, with their original value.

        On every call frame, at the end one of two things can happen:

        - The transaction succeeds. In this case:
            - The CallFrameBackup of the current callframe has to be merged with the backup of its parent, in the following way:
            For every account that's present in the parent backup, do nothing (i.e. keep the one that's already there).
            For every account that's NOT present in the parent backup but is on the child backup, add the child backup to it.
            Do the same for every individual storage slot.
        - The transaction reverts. In this case:
            - Insert into the cache the value of every account on the CallFrameBackup.
            - Insert into the cache the value of every storage slot in every account on the CallFrameBackup.

    */
    pub fn get_account_mut(&mut self, address: Address) -> Result<&mut Account, VMError> {
        if cache::is_account_cached(&self.db.cache, &address) {
            self.backup_account_info(address)?;
            cache::get_account_mut(&mut self.db.cache, &address).ok_or(VMError::Internal(
                crate::errors::InternalError::AccountNotFound,
            ))
        } else {
            let acc = self.db.store.get_account(address)?;
            cache::insert_account(&mut self.db.cache, address, acc);
            self.backup_account_info(address)?;
            cache::get_account_mut(&mut self.db.cache, &address).ok_or(VMError::Internal(
                crate::errors::InternalError::AccountNotFound,
            ))
        }
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

    /// Inserts account to cache backing up the previous state of it in the CacheBackup (if it wasn't already backed up)
    pub fn insert_account(&mut self, address: Address, account: Account) -> Result<(), VMError> {
        self.backup_account_info(address)?;
        let _ = cache::insert_account(&mut self.db.cache, address, account);

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
        let storage_slot_was_cold = self
            .substate
            .touched_storage_slots
            .entry(address)
            .or_default()
            .insert(key);

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
        self.backup_storage_slot(address, key)?;

        let account = self.get_account_mut(address)?;
        account.storage.insert(key, new_value);
        Ok(())
    }

    pub fn backup_storage_slot(&mut self, address: Address, key: H256) -> Result<(), VMError> {
        let value = self.get_storage_value(address, key)?;

        let account_storage_backup = self
            .current_call_frame_mut()?
            .call_frame_backup
            .original_account_storage_slots
            .entry(address)
            .or_insert(HashMap::new());

        account_storage_backup.entry(key).or_insert(value);

        Ok(())
    }

    pub fn backup_account_info(&mut self, address: Address) -> Result<(), VMError> {
        if self.call_frames.is_empty() {
            return Ok(());
        }

        let is_not_backed_up = !self
            .current_call_frame_mut()?
            .call_frame_backup
            .original_accounts_info
            .contains_key(&address);

        if is_not_backed_up {
            let account = cache::get_account(&self.db.cache, &address).ok_or(VMError::Internal(
                crate::errors::InternalError::AccountNotFound,
            ))?;
            let info = account.info.clone();
            let code = account.code.clone();

            self.current_call_frame_mut()?
                .call_frame_backup
                .original_accounts_info
                .insert(
                    address,
                    Account {
                        info,
                        code,
                        storage: HashMap::new(),
                    },
                );
        }

        Ok(())
    }
}
