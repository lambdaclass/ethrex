use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::types::Account;
use ethrex_common::Address;
use ethrex_common::U256;
use keccak_hash::H256;

use crate::call_frame::CallFrameBackup;
use crate::errors::InternalError;
use crate::errors::VMError;
use crate::utils::restore_cache_state;
use crate::vm::Substate;
use crate::vm::VM;

use super::cache;
use super::CacheDB;
use super::Database;

#[derive(Clone)]
pub struct GeneralizedDatabase {
    pub store: Arc<dyn Database>,
    pub cache: CacheDB,
    pub immutable_cache: HashMap<Address, Account>,
    pub tx_backup: Option<CallFrameBackup>,
}

impl GeneralizedDatabase {
    pub fn new(store: Arc<dyn Database>, cache: CacheDB) -> Self {
        Self {
            store,
            cache: cache.clone(),
            immutable_cache: cache,
            tx_backup: None,
        }
    }

    // ================== Account related functions =====================
    /// Gets account, first checking the cache and then the database
    /// (caching in the second case)
    pub fn get_account(&mut self, address: Address) -> Result<&Account, InternalError> {
        if !cache::account_is_cached(&self.cache, &address) {
            let account = self.get_account_from_database(address)?;
            cache::insert_account(&mut self.cache, address, account);
        }
        cache::get_account(&self.cache, &address).ok_or(InternalError::AccountNotFound)
    }

    /// **Accesses to an account's information.**
    ///
    /// Accessed accounts are stored in the `accessed_addresses` set.
    /// Accessed accounts take place in some gas cost computation.
    pub fn access_account(
        &mut self,
        accrued_substate: &mut Substate,
        address: Address,
    ) -> Result<(&Account, bool), InternalError> {
        let address_was_cold = accrued_substate.accessed_addresses.insert(address);
        let account = self.get_account(address)?;

        Ok((account, address_was_cold))
    }

    /// Gets account from storage, storing in Immutable Cache for efficiency when getting AccountUpdates.
    pub fn get_account_from_database(
        &mut self,
        address: Address,
    ) -> Result<Account, InternalError> {
        let account = self.store.get_account(address)?;
        self.immutable_cache.insert(address, account.clone());
        Ok(account)
    }

    /// Gets storage slot from Database, storing in Immutable Cache for efficiency when getting AccountUpdates.
    pub fn get_value_from_database(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<U256, InternalError> {
        let value = self.store.get_storage_value(address, key)?;
        // Account must already be in immutable_cache
        match self.immutable_cache.get_mut(&address) {
            Some(account) => {
                account.storage.insert(key, value);
            }
            None => {
                // If we are fetching the storage of an account it means that we previously fetched the account from database before.
                return Err(InternalError::msg(
                    "Account not found in InMemoryDB when fetching storage",
                ));
            }
        }
        Ok(value)
    }

    /// Gets the transaction backup, if it exists.
    /// It only works if the `BackupHook` was enabled during the transaction execution.
    pub fn get_tx_backup(&self) -> Result<CallFrameBackup, InternalError> {
        self.tx_backup.clone().ok_or(InternalError::Custom(
            "Transaction backup not found. Was BackupHook enabled?".to_string(),
        ))
    }

    /// Undoes the last transaction by restoring the cache state to the state before the transaction.
    pub fn undo_last_transaction(&mut self) -> Result<(), VMError> {
        let tx_backup = self.get_tx_backup()?;
        restore_cache_state(self, tx_backup)?;
        Ok(())
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
    pub fn get_account_mut(&mut self, address: Address) -> Result<&mut Account, InternalError> {
        if cache::is_account_cached(&self.db.cache, &address) {
            self.backup_account_info(address)?;
            cache::get_account_mut(&mut self.db.cache, &address)
                .ok_or(InternalError::AccountNotFound)
        } else {
            let acc = self.db.get_account_from_database(address)?;
            cache::insert_account(&mut self.db.cache, address, acc);
            self.backup_account_info(address)?;
            cache::get_account_mut(&mut self.db.cache, &address)
                .ok_or(InternalError::AccountNotFound)
        }
    }

    pub fn increase_account_balance(
        &mut self,
        address: Address,
        increase: U256,
    ) -> Result<(), InternalError> {
        let account = self.get_account_mut(address)?;
        account.info.balance = account
            .info
            .balance
            .checked_add(increase)
            .ok_or(InternalError::Overflow)?;
        Ok(())
    }

    pub fn decrease_account_balance(
        &mut self,
        address: Address,
        decrease: U256,
    ) -> Result<(), InternalError> {
        let account = self.get_account_mut(address)?;
        account.info.balance = account
            .info
            .balance
            .checked_sub(decrease)
            .ok_or(InternalError::Underflow)?;
        Ok(())
    }

    pub fn transfer(
        &mut self,
        from: Address,
        to: Address,
        value: U256,
    ) -> Result<(), InternalError> {
        self.decrease_account_balance(from, value)?;
        self.increase_account_balance(to, value)?;
        Ok(())
    }

    /// Updates bytecode of given account.
    pub fn update_account_bytecode(
        &mut self,
        address: Address,
        new_bytecode: Bytes,
    ) -> Result<(), InternalError> {
        let account = self.get_account_mut(address)?;
        account.set_code(new_bytecode);
        Ok(())
    }

    // =================== Nonce related functions ======================
    pub fn increment_account_nonce(&mut self, address: Address) -> Result<u64, InternalError> {
        let account = self.get_account_mut(address)?;
        account.info.nonce = account
            .info
            .nonce
            .checked_add(1)
            .ok_or(InternalError::Overflow)?;
        Ok(account.info.nonce)
    }

    /// Inserts account to cache backing up the previous state of it in the CacheBackup (if it wasn't already backed up)
    pub fn insert_account(
        &mut self,
        address: Address,
        account: Account,
    ) -> Result<(), InternalError> {
        self.backup_account_info(address)?;
        let _ = cache::insert_account(&mut self.db.cache, address, account);

        Ok(())
    }

    /// Gets original storage value of an account, caching it if not already cached.
    /// Also saves the original value for future gas calculations.
    pub fn get_original_storage(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<U256, InternalError> {
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
    /// Accessed storage slots are stored in the `accessed_storage_slots` set.
    /// Accessed storage slots take place in some gas cost computation.
    pub fn access_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<(U256, bool), InternalError> {
        // [EIP-2929] - Introduced conditional tracking of accessed storage slots for Berlin and later specs.
        let storage_slot_was_cold = self
            .substate
            .accessed_storage_slots
            .entry(address)
            .or_default()
            .insert(key);

        let storage_slot = self.get_storage_value(address, key)?;

        Ok((storage_slot, storage_slot_was_cold))
    }

    /// Gets storage value of an account, caching it if not already cached.
    pub fn get_storage_value(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<U256, InternalError> {
        if let Some(account) = cache::get_account(&self.db.cache, &address) {
            if let Some(value) = account.storage.get(&key) {
                return Ok(*value);
            }
        } else {
            // When requesting storage of an account we should've previously requested and cached the account
            return Err(InternalError::AccountNotFound);
        }

        let value = self.db.get_value_from_database(address, key)?;

        // Update the account with the fetched value
        let account = self.get_account_mut(address)?;
        account.storage.insert(key, value);

        Ok(value)
    }

    /// Updates storage of an account, caching it if not already cached.
    pub fn update_account_storage(
        &mut self,
        address: Address,
        key: H256,
        new_value: U256,
    ) -> Result<(), InternalError> {
        self.backup_storage_slot(address, key)?;

        let account = self.get_account_mut(address)?;
        account.storage.insert(key, new_value);
        Ok(())
    }

    pub fn backup_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<(), InternalError> {
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

    pub fn backup_account_info(&mut self, address: Address) -> Result<(), InternalError> {
        if self.call_frames.is_empty() {
            return Ok(());
        }

        let is_not_backed_up = !self
            .current_call_frame_mut()?
            .call_frame_backup
            .original_accounts_info
            .contains_key(&address);

        if is_not_backed_up {
            let account = cache::get_account(&self.db.cache, &address)
                .ok_or(InternalError::AccountNotFound)?;
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
