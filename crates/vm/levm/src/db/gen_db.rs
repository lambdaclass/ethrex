use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::Address;
use ethrex_common::U256;

use crate::call_frame::CallFrame;
use crate::errors::InternalError;
use crate::errors::VMError;
use crate::vm::Substate;
use crate::Account;
use crate::AccountInfo;
use std::collections::HashMap;

use super::cache;
use super::error::DatabaseError;
use super::CacheDB;
use super::Database;

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

    /// Gets mutable account, first checking the cache and then the database
    /// (caching in the second case)
    /// This isn't a method of VM because it allows us to use it during VM initialization.
    pub fn get_account_mut<'a>(
        &'a mut self,
        address: Address,
        call_frame: Option<&mut CallFrame>,
    ) -> Result<&'a mut Account, VMError> {
        if !cache::is_account_cached(&self.cache, &address) {
            let account_info = self.store.get_account_info(address)?;
            let account = Account {
                info: account_info,
                storage: HashMap::new(),
            };
            cache::insert_account(&mut self.cache, address, account.clone());
        }

        let original_account = cache::get_account_mut(&mut self.cache, &address)
            .ok_or(VMError::Internal(InternalError::AccountNotFound))?;

        if let Some(call_frame) = call_frame {
            call_frame
                .previous_cache_state
                .entry(address)
                .or_insert_with(|| Some(original_account.clone()));
        };

        Ok(original_account)
    }

    pub fn increase_account_balance(
        &mut self,
        address: Address,
        increase: U256,
        call_frame: Option<&mut CallFrame>,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address, call_frame)?;
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
        call_frame: Option<&mut CallFrame>,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address, call_frame)?;
        account.info.balance = account
            .info
            .balance
            .checked_sub(decrease)
            .ok_or(VMError::BalanceUnderflow)?;
        Ok(())
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

    /// Updates bytecode of given account.
    pub fn update_account_bytecode(
        &mut self,
        address: Address,
        new_bytecode: Bytes,
        call_frame: Option<&mut CallFrame>,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address, call_frame)?;
        account.info.bytecode = new_bytecode;
        Ok(())
    }

    /// Inserts account to cache backing up the previus state of it in the callframe (if it wasn't already backed up)
    pub fn insert_account(
        &mut self,
        address: Address,
        account: Account,
        call_frame: &mut CallFrame,
    ) {
        let previous_account = cache::insert_account(&mut self.cache, address, account);

        call_frame
            .previous_cache_state
            .entry(address)
            .or_insert_with(|| previous_account.as_ref().map(|account| (*account).clone()));
    }

    /// Removes account from cache backing up the previus state of it in the callframe (if it wasn't already backed up)
    pub fn remove_account(&mut self, address: Address, call_frame: &mut CallFrame) {
        let previous_account = cache::remove_account(&mut self.cache, &address);

        call_frame
            .previous_cache_state
            .entry(address)
            .or_insert_with(|| previous_account.as_ref().map(|account| (*account).clone()));
    }
}
