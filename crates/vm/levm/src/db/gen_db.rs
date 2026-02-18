use std::sync::Arc;

use ethrex_common::Address;
use ethrex_common::H256;
use ethrex_common::U256;
use ethrex_common::types::Account;
use ethrex_common::types::Code;
use ethrex_common::types::CodeMetadata;
use ethrex_common::types::block_access_list::{BlockAccessList, BlockAccessListRecorder};
use ethrex_common::utils::ZERO_U256;

use super::Database;
use crate::account::AccountStatus;
use crate::account::LevmAccount;
use crate::call_frame::CallFrameBackup;
use crate::errors::InternalError;
use crate::errors::VMError;
use crate::utils::account_to_levm_account;
use crate::utils::restore_cache_state;
use crate::vm::VM;
pub use ethrex_common::types::AccountUpdate;
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;

pub type CacheDB = FxHashMap<Address, LevmAccount>;

#[derive(Clone)]
pub struct GeneralizedDatabase {
    pub store: Arc<dyn Database>,
    pub current_accounts_state: CacheDB,
    pub initial_accounts_state: CacheDB,
    pub codes: FxHashMap<H256, Code>,
    pub code_metadata: FxHashMap<H256, CodeMetadata>,
    pub tx_backup: Option<CallFrameBackup>,
    /// Optional BAL recorder for EIP-7928 Block Access List recording.
    pub bal_recorder: Option<BlockAccessListRecorder>,
}

impl GeneralizedDatabase {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self {
            store,
            current_accounts_state: Default::default(),
            initial_accounts_state: Default::default(),
            tx_backup: None,
            codes: Default::default(),
            code_metadata: Default::default(),
            bal_recorder: None,
        }
    }

    /// Enables BAL recording for EIP-7928.
    /// After enabling, state changes will be recorded during execution.
    pub fn enable_bal_recording(&mut self) {
        self.bal_recorder = Some(BlockAccessListRecorder::new());
    }

    /// Disables BAL recording.
    pub fn disable_bal_recording(&mut self) {
        self.bal_recorder = None;
    }

    /// Sets the current block access index for BAL recording per EIP-7928 spec (uint16).
    /// Call this before each transaction or phase.
    pub fn set_bal_index(&mut self, index: u16) {
        if let Some(recorder) = &mut self.bal_recorder {
            recorder.set_block_access_index(index);
        }
    }

    /// Takes the BAL recorder and builds the final BlockAccessList.
    /// Returns None if recording was not enabled.
    pub fn take_bal(&mut self) -> Option<BlockAccessList> {
        self.bal_recorder.take().map(|recorder| recorder.build())
    }

    /// Returns a mutable reference to the BAL recorder if enabled.
    pub fn bal_recorder_mut(&mut self) -> Option<&mut BlockAccessListRecorder> {
        self.bal_recorder.as_mut()
    }

    /// Only used within Levm Runner, where the accounts already have all the storage pre-loaded, not used in real case scenarios.
    pub fn new_with_account_state(
        store: Arc<dyn Database>,
        current_accounts_state: FxHashMap<Address, Account>,
    ) -> Self {
        let mut codes: FxHashMap<H256, Code> = Default::default();
        let levm_accounts: FxHashMap<Address, LevmAccount> = current_accounts_state
            .into_iter()
            .map(|(address, account)| {
                let (levm_account, code) = account_to_levm_account(account);
                codes.insert(levm_account.info.code_hash, code);
                (address, levm_account)
            })
            .collect();
        Self {
            store,
            current_accounts_state: levm_accounts.clone(),
            initial_accounts_state: levm_accounts,
            tx_backup: None,
            codes,
            code_metadata: Default::default(),
            bal_recorder: None,
        }
    }

    // ================== Account related functions =====================
    /// Loads account
    /// If it's the first time it's loaded store it in `initial_accounts_state` and also cache it in `current_accounts_state` for making changes to it
    fn load_account(&mut self, address: Address) -> Result<&mut LevmAccount, InternalError> {
        match self.current_accounts_state.entry(address) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                if let Some(account) = self.initial_accounts_state.get(&address) {
                    return Ok(entry.insert(account.clone()));
                }
                let state = self.store.get_account_state(address)?;
                let account = LevmAccount::from(state);
                self.initial_accounts_state.insert(address, account.clone());
                Ok(entry.insert(account))
            }
        }
    }

    /// Gets reference of an account
    pub fn get_account(&mut self, address: Address) -> Result<&LevmAccount, InternalError> {
        Ok(self.load_account(address)?)
    }

    /// Gets mutable reference of an account
    /// Warning: Use directly only if outside of the EVM, otherwise use `vm.get_account_mut` because it contemplates call frame backups.
    pub fn get_account_mut(&mut self, address: Address) -> Result<&mut LevmAccount, InternalError> {
        let acc = self.load_account(address)?;
        acc.mark_modified();
        Ok(acc)
    }

    /// Gets code immutably given the code hash.
    /// Use this only inside of the VM, when we don't surely know if the code is in the cache or not
    /// But e.g. in `get_state_transitions` just do `db.codes.get(code_hash)` because we know for sure code is there.
    pub fn get_code(&mut self, code_hash: H256) -> Result<&Code, InternalError> {
        match self.codes.entry(code_hash) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let code = self.store.get_account_code(code_hash)?;
                Ok(entry.insert(code))
            }
        }
    }

    /// Shortcut for getting the code when we only have the address of an account and we don't need anything else.
    pub fn get_account_code(&mut self, address: Address) -> Result<&Code, InternalError> {
        let code_hash = self.get_account(address)?.info.code_hash;
        self.get_code(code_hash)
    }

    /// Gets code metadata immutably given the code hash.
    pub fn get_code_metadata(&mut self, code_hash: H256) -> Result<&CodeMetadata, InternalError> {
        match self.code_metadata.entry(code_hash) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                // First ensure code is loaded into cache by calling get_code
                // This handles witness fallbacks and other code loading logic correctly
                #[expect(clippy::as_conversions, reason = "same sized types (on 64bit)")]
                let code_length = {
                    // Note: `self.get_code(code_hash)` has been inlined due to mutability borrow issues.
                    //   To avoid this inlinement, self.get_code has to be moved into `self.codes` so that it's called
                    //   like this: `self.codes.get(code_hash)`.
                    let code = match self.codes.entry(code_hash) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            entry.insert(self.store.get_account_code(code_hash)?)
                        }
                    };

                    code.bytecode.len() as u64
                };

                let metadata = CodeMetadata {
                    length: code_length,
                };

                // Insert into cache and return reference
                Ok(entry.insert(metadata))
            }
        }
    }

    /// Convenience method to get code length by address (optimized for EXTCODESIZE).
    pub fn get_code_length(&mut self, address: Address) -> Result<usize, InternalError> {
        use ethrex_common::constants::EMPTY_KECCACK_HASH;

        let code_hash = self.get_account(address)?.info.code_hash;
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(0);
        }
        let metadata = self.get_code_metadata(code_hash)?;
        #[expect(clippy::as_conversions, reason = "same sized types (on 64bit)")]
        Ok(metadata.length as usize)
    }

    /// Gets storage slot from Database, storing in initial_accounts_state for efficiency when getting AccountUpdates.
    fn get_value_from_database(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<U256, InternalError> {
        let value = self.store.get_storage_value(address, key)?;
        // Account must already be in initial_accounts_state
        match self.initial_accounts_state.get_mut(&address) {
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
        self.tx_backup.clone().ok_or_else(|| {
            InternalError::Custom(
                "Transaction backup not found. Was BackupHook enabled?".to_string(),
            )
        })
    }

    /// Undoes the last transaction by restoring the cache state to the state before the transaction.
    pub fn undo_last_transaction(&mut self) -> Result<(), VMError> {
        let tx_backup = self.get_tx_backup()?;
        restore_cache_state(self, tx_backup)?;
        Ok(())
    }

    pub fn get_state_transitions(&mut self) -> Result<Vec<AccountUpdate>, VMError> {
        let mut account_updates: Vec<AccountUpdate> = vec![];
        for (address, new_state_account) in self.current_accounts_state.iter() {
            if new_state_account.is_unmodified() {
                // Skip processing account that we know wasn't mutably accessed during execution
                continue;
            }
            // In case the account is not in immutable_cache (rare) we search for it in the actual database.
            let initial_state_account =
                self.initial_accounts_state.get(address).ok_or_else(|| {
                    VMError::Internal(InternalError::Custom(format!(
                        "Failed to get account {address} from immutable cache",
                    )))
                })?;

            let mut acc_info_updated = false;
            let mut storage_updated = false;

            // 1. Account Info has been updated if balance, nonce or bytecode changed.
            if initial_state_account.info.balance != new_state_account.info.balance {
                acc_info_updated = true;
            }

            if initial_state_account.info.nonce != new_state_account.info.nonce {
                acc_info_updated = true;
            }

            let code = if initial_state_account.info.code_hash != new_state_account.info.code_hash {
                acc_info_updated = true;
                // code should be in `codes`
                Some(
                    self.codes
                        .get(&new_state_account.info.code_hash)
                        .ok_or_else(|| {
                            VMError::Internal(InternalError::Custom(format!(
                                "Failed to get code for account {address}"
                            )))
                        })?,
                )
            } else {
                None
            };

            // Account will have only its storage removed if it was Destroyed and then modified
            // Edge cases that can make this true:
            //   1. Account was destroyed and created again afterwards.
            //   2. Account was destroyed but then was sent ETH, so it's not going to be completely removed from the trie.
            let removed_storage = new_state_account.status == AccountStatus::DestroyedModified;

            // 2. Storage has been updated if the current value is different from the one before execution.
            let mut added_storage: FxHashMap<_, _> = Default::default();

            for (key, new_value) in &new_state_account.storage {
                let old_value = if !removed_storage {
                    initial_state_account.storage.get(key).ok_or_else(|| { VMError::Internal(InternalError::Custom(format!("Failed to get old value from account's initial storage for address: {address:?}. For key: {key:?}")))})?
                } else {
                    // There's not an "old value" if the contract was destroyed and re-created.
                    &ZERO_U256
                };

                if new_value != old_value {
                    added_storage.insert(*key, *new_value);
                    storage_updated = true;
                }
            }

            let info = if acc_info_updated {
                Some(new_state_account.info.clone())
            } else {
                None
            };

            // "At the end of the transaction, any account touched by the execution of that transaction which is now empty SHALL instead become non-existent (i.e. deleted)."
            // ethrex is a post-Merge client, empty accounts have already been pruned from the trie on Mainnet by the Merge (see EIP-161), so we won't have any empty accounts in the trie.
            let was_empty = initial_state_account.is_empty();
            let removed = new_state_account.is_empty() && !was_empty;

            if !removed && !acc_info_updated && !storage_updated && !removed_storage {
                // Account hasn't been updated
                continue;
            }

            let account_update = AccountUpdate {
                address: *address,
                removed,
                info,
                code: code.cloned(),
                added_storage,
                removed_storage,
            };

            account_updates.push(account_update);
        }
        self.initial_accounts_state.clear();
        self.current_accounts_state.clear();
        self.codes.clear();
        self.code_metadata.clear();
        Ok(account_updates)
    }

    pub fn get_state_transitions_tx(&mut self) -> Result<Vec<AccountUpdate>, VMError> {
        let mut account_updates: Vec<AccountUpdate> = vec![];
        for (address, new_state_account) in self.current_accounts_state.drain() {
            if new_state_account.is_unmodified() {
                // Skip processing account that we know wasn't mutably accessed during execution
                continue;
            }
            // [LIE] In case the account is not in immutable_cache (rare) we search for it in the actual database.
            let initial_state_account =
                self.initial_accounts_state.get(&address).ok_or_else(|| {
                    VMError::Internal(InternalError::Custom(format!(
                        "Failed to get account {address} from immutable cache",
                    )))
                })?;

            let mut acc_info_updated = false;
            let mut storage_updated = false;

            // 1. Account Info has been updated if balance, nonce or bytecode changed.
            if initial_state_account.info.balance != new_state_account.info.balance {
                acc_info_updated = true;
            }

            if initial_state_account.info.nonce != new_state_account.info.nonce {
                acc_info_updated = true;
            }

            let code = if initial_state_account.info.code_hash != new_state_account.info.code_hash {
                acc_info_updated = true;
                // code should be in `codes`
                Some(
                    self.codes
                        .get(&new_state_account.info.code_hash)
                        .cloned()
                        .ok_or_else(|| {
                            VMError::Internal(InternalError::Custom(format!(
                                "Failed to get code for account {address}"
                            )))
                        })?,
                )
            } else {
                None
            };

            // Account will have only its storage removed if it was Destroyed and then modified
            // Edge cases that can make this true:
            //   1. Account was destroyed and created again afterwards.
            //   2. Account was destroyed but then was sent ETH, so it's not going to be completely removed from the trie.
            let removed_storage = new_state_account.status == AccountStatus::DestroyedModified;

            // 2. Storage has been updated if the current value is different from the one before execution.
            let mut added_storage: FxHashMap<_, _> = Default::default();

            for (key, new_value) in &new_state_account.storage {
                let old_value = if !removed_storage {
                    initial_state_account.storage.get(key).ok_or_else(|| { VMError::Internal(InternalError::Custom(format!("Failed to get old value from account's initial storage for address: {address}")))})?
                } else {
                    // There's not an "old value" if the contract was destroyed and re-created.
                    &ZERO_U256
                };

                if new_value != old_value {
                    added_storage.insert(*key, *new_value);
                    storage_updated = true;
                }
            }

            let info = acc_info_updated.then(|| new_state_account.info.clone());

            // "At the end of the transaction, any account touched by the execution of that transaction which is now empty SHALL instead become non-existent (i.e. deleted)."
            // ethrex is a post-Merge client, empty accounts have already been pruned from the trie on Mainnet by the Merge (see EIP-161), so we won't have any empty accounts in the trie.
            let was_empty = initial_state_account.is_empty();
            let removed = new_state_account.is_empty() && !was_empty;

            if !removed && !acc_info_updated && !storage_updated && !removed_storage {
                // Account hasn't been updated
                continue;
            }

            self.initial_accounts_state
                .insert(address, new_state_account);

            let account_update = AccountUpdate {
                address,
                removed,
                info,
                code,
                added_storage,
                removed_storage,
            };

            account_updates.push(account_update);
        }
        Ok(account_updates)
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
    pub fn get_account_mut(&mut self, address: Address) -> Result<&mut LevmAccount, InternalError> {
        // Take backup BEFORE mark_modified so the original status is preserved
        let account = self.db.get_account(address)?;
        self.current_call_frame
            .call_frame_backup
            .backup_account_info(address, account)?;

        let account = self.db.get_account_mut(address)?;
        Ok(account)
    }

    pub fn increase_account_balance(
        &mut self,
        address: Address,
        increase: U256,
    ) -> Result<(), InternalError> {
        let account = self.get_account_mut(address)?;

        // Get initial balance BEFORE modification (avoids duplicate lookup)
        let initial_balance = account.info.balance;

        // Modify balance
        account.info.balance = account
            .info
            .balance
            .checked_add(increase)
            .ok_or(InternalError::Overflow)?;
        let new_balance = account.info.balance;

        // Record initial and changed balance for BAL
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.set_initial_balance(address, initial_balance);
            recorder.record_balance_change(address, new_balance);
        }

        Ok(())
    }

    pub fn decrease_account_balance(
        &mut self,
        address: Address,
        decrease: U256,
    ) -> Result<(), InternalError> {
        let account = self.get_account_mut(address)?;

        // Get initial balance BEFORE modification (avoids duplicate lookup)
        let initial_balance = account.info.balance;

        // Modify balance
        account.info.balance = account
            .info
            .balance
            .checked_sub(decrease)
            .ok_or(InternalError::Underflow)?;
        let new_balance = account.info.balance;

        // Record initial and changed balance for BAL
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.set_initial_balance(address, initial_balance);
            recorder.record_balance_change(address, new_balance);
        }

        Ok(())
    }

    pub fn transfer(
        &mut self,
        from: Address,
        to: Address,
        value: U256,
    ) -> Result<(), InternalError> {
        if value != U256::zero() {
            self.decrease_account_balance(from, value)?;
            self.increase_account_balance(to, value)?;
        }

        Ok(())
    }

    /// Updates bytecode of given account.
    pub fn update_account_bytecode(
        &mut self,
        address: Address,
        new_bytecode: Code,
    ) -> Result<(), InternalError> {
        // Record code change for BAL
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            // Capture initial code BEFORE recording the change.
            // This is needed for:
            // 1. Distinguishing CREATE empty code vs delegation clear
            // 2. Net-zero code change detection (e.g., delegate then reset in same tx)
            let current_code_bytes = self
                .db
                .current_accounts_state
                .get(&address)
                .and_then(|account| self.db.codes.get(&account.info.code_hash))
                .map(|c| c.bytecode.clone())
                .unwrap_or_default();
            let has_code = !current_code_bytes.is_empty();
            recorder.capture_initial_code_presence(address, has_code);
            recorder.set_initial_code(address, current_code_bytes);
            recorder.record_code_change(address, new_bytecode.bytecode.clone());
        }

        let acc = self.get_account_mut(address)?;
        let code_hash = new_bytecode.hash;
        acc.info.code_hash = new_bytecode.hash;
        self.db.codes.entry(code_hash).or_insert(new_bytecode);
        Ok(())
    }

    // =================== Nonce related functions ======================
    /// Increments the nonce of the given account.
    /// Per EIP-7928, nonce changes are recorded for:
    /// - EOA senders
    /// - Contracts performing CREATE/CREATE2
    /// - Deployed contracts
    /// - EIP-7702 authorities
    pub fn increment_account_nonce(&mut self, address: Address) -> Result<u64, InternalError> {
        let account = self.get_account_mut(address)?;
        account.info.nonce = account
            .info
            .nonce
            .checked_add(1)
            .ok_or(InternalError::Overflow)?;
        let new_nonce = account.info.nonce;

        // Record nonce change for BAL
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.record_nonce_change(address, new_nonce);
        }

        Ok(new_nonce)
    }

    /// Gets original storage value of an account, caching it if not already cached.
    /// Also saves the original value for future gas calculations.
    pub fn get_original_storage(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<U256, InternalError> {
        if let Some(value) = self.storage_original_values.get(&(address, key)) {
            return Ok(*value);
        }

        let value = self.get_storage_value(address, key)?;
        self.storage_original_values.insert((address, key), value);
        Ok(value)
    }

    /// Accesses to an account's storage slot and returns the value in it.
    ///
    /// Accessed storage slots are stored in the `accessed_storage_slots` set.
    /// Accessed storage slots take place in some gas cost computation.
    ///
    /// Note: This function does NOT record to BAL. Per EIP-7928, BAL recording
    /// must happen after gas checks pass. Use `record_storage_slot_to_bal()`
    /// separately after the gas check succeeds.
    pub fn access_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<(U256, bool), InternalError> {
        // [EIP-2929] - Introduced conditional tracking of accessed storage slots for Berlin and later specs.
        let storage_slot_was_cold = !self.substate.add_accessed_slot(address, key);

        let storage_slot = self.get_storage_value(address, key)?;

        // Note: BAL recording is NOT done here per EIP-7928.
        // "If pre-state validation fails, the target is never accessed and must not appear in BAL."
        // Call record_storage_slot_to_bal() after gas check passes.

        Ok((storage_slot, storage_slot_was_cold))
    }

    /// Records a storage slot read to BAL after gas checks have passed.
    /// Per EIP-7928: "If pre-state validation fails, the target is never accessed and must not appear in BAL."
    /// This function should be called AFTER the gas check succeeds.
    pub fn record_storage_slot_to_bal(&mut self, address: Address, key: U256) {
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.record_storage_read(address, key);
        }
    }

    /// Gets storage value of an account, caching it if not already cached.
    #[inline(always)]
    pub fn get_storage_value(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<U256, InternalError> {
        if let Some(account) = self.db.current_accounts_state.get(&address) {
            if let Some(value) = account.storage.get(&key) {
                return Ok(*value);
            }
            // If the account was destroyed and then created then we cannot rely on the DB to obtain storage values
            if account.status == AccountStatus::DestroyedModified {
                return Ok(U256::zero());
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
        slot_key: U256,
        new_value: U256,
        current_value: U256,
    ) -> Result<(), InternalError> {
        self.backup_storage_slot(address, key, current_value)?;

        // Record storage change for BAL (EIP-7928).
        // SSTORE that changes the value (new != current) → storage write.
        // SSTORE with same value (new == current) → storage read (no actual mutation).
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            if new_value != current_value {
                // Record original value before first write. If final value equals original
                // after all tx operations, the slot becomes a read per EIP-7928 net-zero filtering.
                // This captures the value BEFORE the first write in this transaction
                recorder.capture_pre_storage(address, slot_key, current_value);
                // Actual write
                recorder.record_storage_write(address, slot_key, new_value);
            } else {
                // No-op write (post == pre) - record as read per EIP-7928
                recorder.record_storage_read(address, slot_key);
            }
        }

        let account = self.get_account_mut(address)?;
        account.storage.insert(key, new_value);
        Ok(())
    }

    pub fn backup_storage_slot(
        &mut self,
        address: Address,
        key: H256,
        current_value: U256,
    ) -> Result<(), InternalError> {
        self.current_call_frame
            .call_frame_backup
            .original_account_storage_slots
            .entry(address)
            .or_default()
            .entry(key)
            .or_insert(current_value);

        Ok(())
    }
}
