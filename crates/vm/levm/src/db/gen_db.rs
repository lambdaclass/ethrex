use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::Address;
use ethrex_common::U256;

use crate::errors::InternalError;
use crate::errors::VMError;
use crate::vm::Substate;
use crate::vm::VM;
use crate::Account;
use crate::AccountInfo;
use std::collections::HashMap;

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
    pub fn get_account(&mut self, address: Address) -> Result<Account, DatabaseError> {
        match cache::get_account(&self.cache, &address) {
            Some(acc) => Ok(acc.clone()),
            None => {
                let account_info = self.store.get_account_info(address)?;
                let account = Account {
                    info: account_info,
                    storage: HashMap::new(),
                };
                cache::insert_account(&mut self.cache, address, account.clone());
                Ok(account)
            }
        }
    }

    /// Gets account without pushing it to the cache
    pub fn get_account_no_push_cache(&self, address: Address) -> Result<Account, DatabaseError> {
        match cache::get_account(&self.cache, &address) {
            Some(acc) => Ok(acc.clone()),
            None => {
                let account_info = self.store.get_account_info(address)?;
                Ok(Account {
                    info: account_info,
                    storage: HashMap::new(),
                })
            }
        }
    }

    /// **Accesses to an account's information.**
    ///
    /// Accessed accounts are stored in the `touched_accounts` set.
    /// Accessed accounts take place in some gas cost computation.
    pub fn access_account(
        &mut self,
        accrued_substate: &mut Substate,
        address: Address,
    ) -> Result<(AccountInfo, bool), DatabaseError> {
        let address_was_cold = accrued_substate.touched_accounts.insert(address);
        let account = match cache::get_account(&self.cache, &address) {
            Some(account) => account.info.clone(),
            None => self.store.get_account_info(address)?,
        };
        Ok((account, address_was_cold))
    }
}

impl<'a> VM<'a> {
    // ================== Account related functions =====================

    pub fn get_account_mut(&mut self, address: Address) -> Result<&mut Account, VMError> {
        if !cache::is_account_cached(&self.db.cache, &address) {
            let account_info = self.db.store.get_account_info(address)?;
            let account = Account {
                info: account_info,
                storage: HashMap::new(),
            };
            cache::insert_account(&mut self.db.cache, address, account.clone());
        }

        let backup_account = cache::get_account(&self.db.cache, &address)
            .ok_or(VMError::Internal(InternalError::AccountNotFound))?
            .clone();
        self.current_call_frame_mut()?
            .cache_backup
            .entry(address)
            .or_insert_with(|| Some(backup_account));

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
        account.info.bytecode = new_bytecode;
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

    pub fn decrement_account_nonce(&mut self, address: Address) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.info.nonce = account
            .info
            .nonce
            .checked_sub(1)
            .ok_or(VMError::NonceUnderflow)?;
        Ok(())
    }

    /// Inserts account to cache backing up the previus state of it in the CacheBackup (if it wasn't already backed up)
    pub fn insert_account(&mut self, address: Address, account: Account) -> Result<(), VMError> {
        let previous_account = cache::insert_account(&mut self.db.cache, address, account);

        self.current_call_frame_mut()?
            .cache_backup
            .entry(address)
            .or_insert_with(|| previous_account.as_ref().map(|account| (*account).clone()));

        Ok(())
    }

    /// Removes account from cache backing up the previus state of it in the CacheBackup (if it wasn't already backed up)
    pub fn remove_account(&mut self, address: Address) -> Result<(), VMError> {
        let previous_account = cache::remove_account(&mut self.db.cache, &address);

        self.current_call_frame_mut()?
            .cache_backup
            .entry(address)
            .or_insert_with(|| previous_account.as_ref().map(|account| (*account).clone()));

        Ok(())
    }
}
