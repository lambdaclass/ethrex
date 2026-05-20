use std::sync::Arc;

use ethrex_common::Address;
use ethrex_common::H256;
use ethrex_common::U256;
use ethrex_common::types::Account;
use ethrex_common::types::Code;
use ethrex_common::types::CodeMetadata;
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use ethrex_common::types::block_access_list::SlotChange;
use ethrex_common::types::block_access_list::{
    BalAddressIndex, BlockAccessList, BlockAccessListRecorder,
};
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
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::hash_map::Entry;

pub type CacheDB = FxHashMap<Address, LevmAccount>;

/// Per-tx BAL cursor for lazy on-read prefix materialization.
/// `bal_index = tx_idx + 1`; cursor's effective max_idx is `bal_index - 1`,
/// matching `seed_db_from_bal`'s `max_idx = tx_idx` semantics.
#[derive(Clone)]
pub struct LazyBalCursor {
    pub bal: Arc<BlockAccessList>,
    pub bal_index: u32,
    pub index: Arc<BalAddressIndex>,
}

/// Apply balance, nonce, and code fields from BAL for a single account into `db`.
///
/// Returns `true` if any info field was applied; `false` if all field positions
/// were 0 (no info changes for this account at indices <= max_idx).
/// Does NOT touch `account.storage`.
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
pub fn seed_one_address_info_from_bal(
    db: &mut GeneralizedDatabase,
    bal: &BlockAccessList,
    acct_idx: usize,
    max_idx: u32,
) -> Result<bool, InternalError> {
    use ethrex_common::types::AccountInfo;

    let acct_changes = bal
        .accounts()
        .get(acct_idx)
        .ok_or(InternalError::AccountNotFound)?;
    let addr = acct_changes.address;

    let balance_pos = acct_changes
        .balance_changes
        .partition_point(|c| c.block_access_index <= max_idx);
    let nonce_pos = acct_changes
        .nonce_changes
        .partition_point(|c| c.block_access_index <= max_idx);
    let code_pos = acct_changes
        .code_changes
        .partition_point(|c| c.block_access_index <= max_idx);

    if balance_pos == 0 && nonce_pos == 0 && code_pos == 0 {
        return Ok(false);
    }

    // Compute code update before borrowing acc (borrow checker: can't access
    // db.codes while acc holds a mutable borrow of db).
    let code_update = if code_pos > 0 {
        let entry = acct_changes
            .code_changes
            .get(code_pos.saturating_sub(1))
            .ok_or(InternalError::AccountNotFound)?;
        Some(code_from_bal(&entry.new_code))
    } else {
        None
    };

    // When BAL covers all account info fields (balance + nonce + code), insert
    // a default LevmAccount directly to skip the store/shared_base lookup.
    // For partial coverage, load from store to fill missing fields.
    let has_all_info = balance_pos > 0 && nonce_pos > 0 && code_pos > 0;
    if has_all_info {
        use ethrex_common::constants::EMPTY_KECCACK_HASH;
        let balance = acct_changes
            .balance_changes
            .get(balance_pos.saturating_sub(1))
            .ok_or(InternalError::AccountNotFound)?
            .post_balance;
        let nonce = acct_changes
            .nonce_changes
            .get(nonce_pos.saturating_sub(1))
            .ok_or(InternalError::AccountNotFound)?
            .post_nonce;
        let code_hash = code_update
            .as_ref()
            .map(|(h, _)| *h)
            .unwrap_or(*EMPTY_KECCACK_HASH);
        let acc = db
            .current_accounts_state
            .entry(addr)
            .or_insert_with(|| LevmAccount {
                info: AccountInfo::default(),
                storage: FxHashMap::default(),
                has_storage: false,
                status: AccountStatus::Modified,
                exists: true,
            });
        acc.info.balance = balance;
        acc.info.nonce = nonce;
        acc.info.code_hash = code_hash;
        acc.mark_modified();
    } else {
        db.get_account(addr)
            .map_err(|e| InternalError::Custom(format!("seed_db_from_bal load: {e}")))?;
        let acc = db
            .get_account_mut(addr)
            .map_err(|e| InternalError::Custom(format!("seed bal: {e}")))?;

        if balance_pos > 0
            && let Some(entry) = acct_changes
                .balance_changes
                .get(balance_pos.saturating_sub(1))
        {
            acc.info.balance = entry.post_balance;
        }
        if nonce_pos > 0
            && let Some(entry) = acct_changes.nonce_changes.get(nonce_pos.saturating_sub(1))
        {
            acc.info.nonce = entry.post_nonce;
        }
        if let Some((hash, _)) = &code_update {
            acc.info.code_hash = *hash;
        }
    }

    // Insert code object after acc borrow is released.
    if let Some((hash, Some(code_obj))) = code_update {
        db.codes.entry(hash).or_insert(code_obj);
    }

    Ok(true)
}

/// Select the post-value of a single `SlotChange` up to `max_idx`.
///
/// Pure read; returns `Some(value)` if any `slot_changes` entry has
/// `block_access_index <= max_idx`, `None` otherwise.
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
pub fn post_value_at_or_before(sc: &SlotChange, max_idx: u32) -> Option<U256> {
    let pos = sc
        .slot_changes
        .partition_point(|c| c.block_access_index <= max_idx);
    sc.slot_changes
        .get(pos.saturating_sub(1))
        .filter(|_| pos > 0)
        .map(|c| c.post_value)
}

/// Read the post-value of a single storage slot from the BAL up to `max_idx`.
///
/// O(1) slot resolution via the precomputed `slot_idx_by_account` map in
/// `BalAddressIndex`. Pure read; does not touch `db`.
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
pub fn seed_one_storage_slot_from_bal(
    bal: &BlockAccessList,
    index: &BalAddressIndex,
    acct_idx: usize,
    key: H256,
    max_idx: u32,
) -> Option<U256> {
    let acct_changes = bal.accounts().get(acct_idx)?;
    let slot_map = index.slot_idx_by_account.get(acct_idx)?;
    let sc_idx = *slot_map.get(&key)?;
    let sc = acct_changes.storage_changes.get(sc_idx)?;
    post_value_at_or_before(sc, max_idx)
}

/// Compute code hash and optional `Code` object from raw bytecode in a BAL entry.
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
pub fn code_from_bal(new_code: &bytes::Bytes) -> (H256, Option<Code>) {
    use ethrex_common::constants::EMPTY_KECCACK_HASH;
    if new_code.is_empty() {
        (*EMPTY_KECCACK_HASH, None)
    } else {
        let code_obj = Code::from_bytecode(new_code.clone(), &ethrex_crypto::NativeCrypto);
        let hash = code_obj.hash;
        (hash, Some(code_obj))
    }
}

#[derive(Clone)]
pub struct GeneralizedDatabase {
    pub store: Arc<dyn Database>,
    pub current_accounts_state: CacheDB,
    pub initial_accounts_state: CacheDB,
    /// Shared read-only base state (pre-block snapshot of system-touched addresses for
    /// parallel groups, captured from `initial_accounts_state` after `prepare_block`).
    /// Checked on `load_account` AFTER the `lazy_bal` hook so the BAL overlay (which
    /// includes system-call effects at idx 0) takes precedence for any address the BAL
    /// covers. Accounts are cloned into `initial_accounts_state` on first access.
    pub shared_base: Option<Arc<CacheDB>>,
    pub codes: FxHashMap<H256, Code>,
    pub code_metadata: FxHashMap<H256, CodeMetadata>,
    pub tx_backup: Option<CallFrameBackup>,
    /// Optional BAL recorder for EIP-7928 Block Access List recording.
    pub bal_recorder: Option<BlockAccessListRecorder>,
    /// When true, skip cloning accounts into `initial_accounts_state` on load.
    /// Used for parallel per-tx DBs where `get_state_transitions_tx` is never called
    /// (state transitions come from BAL instead).
    skip_initial_tracking: bool,
    /// Optional tracker for BAL validation: records addresses accessed via load_account.
    /// Enabled only during parallel execution to detect extraneous BAL pure-access entries.
    pub accessed_accounts: Option<FxHashSet<Address>>,
    /// Optional BAL cursor for lazy per-read prefix materialization.
    /// When set, account loads and storage reads consult the BAL before hitting the store.
    pub lazy_bal: Option<LazyBalCursor>,
}

impl GeneralizedDatabase {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self {
            store,
            current_accounts_state: Default::default(),
            initial_accounts_state: Default::default(),
            shared_base: None,
            tx_backup: None,
            codes: Default::default(),
            code_metadata: Default::default(),
            bal_recorder: None,
            skip_initial_tracking: false,
            accessed_accounts: None,
            lazy_bal: None,
        }
    }

    /// Creates a new GeneralizedDatabase with a shared read-only base state.
    /// Used for parallel execution groups that share post-system-call state.
    /// Skips initial_accounts_state tracking since parallel per-tx DBs never
    /// call get_state_transitions_tx (state comes from BAL instead).
    pub fn new_with_shared_base(store: Arc<dyn Database>, shared_base: Arc<CacheDB>) -> Self {
        Self::new_with_shared_base_and_capacity(store, shared_base, 0)
    }

    /// Like `new_with_shared_base` but pre-allocates account/code maps to
    /// `capacity` entries, avoiding rehashing during BAL seeding.
    pub fn new_with_shared_base_and_capacity(
        store: Arc<dyn Database>,
        shared_base: Arc<CacheDB>,
        capacity: usize,
    ) -> Self {
        Self {
            store,
            current_accounts_state: FxHashMap::with_capacity_and_hasher(
                capacity,
                Default::default(),
            ),
            initial_accounts_state: Default::default(),
            shared_base: Some(shared_base),
            tx_backup: None,
            codes: FxHashMap::with_capacity_and_hasher(capacity / 4, Default::default()),
            code_metadata: Default::default(),
            bal_recorder: None,
            skip_initial_tracking: true,
            accessed_accounts: None,
            lazy_bal: None,
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

    /// Sets the current block access index for BAL recording per EIP-7928 spec (uint32).
    /// Call this before each transaction or phase.
    pub fn set_bal_index(&mut self, index: u32) {
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
            shared_base: None,
            tx_backup: None,
            codes,
            code_metadata: Default::default(),
            bal_recorder: None,
            skip_initial_tracking: false,
            accessed_accounts: None,
            lazy_bal: None,
        }
    }

    // ================== Account related functions =====================
    /// Loads account
    /// If it's the first time it's loaded store it in `initial_accounts_state` and also cache it in `current_accounts_state` for making changes to it
    fn load_account(&mut self, address: Address) -> Result<&mut LevmAccount, InternalError> {
        if let Some(tracker) = &mut self.accessed_accounts {
            tracker.insert(address);
        }

        // Fast path: already cached.
        if self.current_accounts_state.contains_key(&address) {
            return self
                .current_accounts_state
                .get_mut(&address)
                .ok_or(InternalError::AccountNotFound);
        }

        // Initial-state fast path.
        if let Some(account) = self.initial_accounts_state.get(&address) {
            let clone = account.clone();
            return Ok(self.current_accounts_state.entry(address).or_insert(clone));
        }

        // Lazy-BAL hook: if the cursor finds this address, materialize info from the BAL
        // before consulting `shared_base` or the store.
        //
        // Ordering matters: `shared_base` holds the pre-block snapshot of system-touched
        // addresses, but the canonical pre-state for tx N is the BAL prefix up to its
        // `bal_index` (= system-call effects at idx 0 plus all prior txs). If `shared_base`
        // were consulted first for an address it covers, the BAL overlay would be skipped
        // and tx N would observe stale balance/nonce/code (consensus bug for system-touched
        // predeploys mutated by a prior tx in the same block).
        //
        // We `.take()` the cursor out of `self.lazy_bal` before calling
        // `seed_one_address_info_from_bal`. For partial-coverage accounts (e.g. balance-only
        // change with no nonce/code) the helper calls `db.get_account(addr)` internally to
        // load the base state before overlaying. If `self.lazy_bal` were still `Some(...)`
        // at that point, `get_account` → `load_account` would re-enter this same block and
        // recurse infinitely. Taking the cursor out breaks the cycle: the inner call sees
        // `lazy_bal = None` and falls through to `shared_base`/store. We restore the cursor
        // unconditionally afterward (even on error) so the outer caller still sees it.
        #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
        {
            let cursor_opt = self.lazy_bal.take();
            let helper_result = if let Some(cursor) = cursor_opt.as_ref() {
                debug_assert!(
                    cursor.bal_index >= 1,
                    "LazyBalCursor bal_index must be >= 1"
                );
                let max_idx = cursor.bal_index.saturating_sub(1);
                if let Some(&acct_idx) = cursor.index.addr_to_idx.get(&address) {
                    Some(
                        seed_one_address_info_from_bal(self, &cursor.bal, acct_idx, max_idx)
                            .map(|_| true),
                    )
                } else {
                    None
                }
            } else {
                None
            };
            // Restore the cursor before propagating any error or returning.
            self.lazy_bal = cursor_opt;
            if let Some(result) = helper_result {
                result.map_err(|e| InternalError::Custom(format!("lazy_bal seed: {e}")))?;
                if self.current_accounts_state.contains_key(&address) {
                    return self
                        .current_accounts_state
                        .get_mut(&address)
                        .ok_or(InternalError::AccountNotFound);
                }
            }
        }

        // Check shared_base (read-only pre-block snapshot) before hitting store.
        if let Some(ref base) = self.shared_base
            && let Some(account) = base.get(&address)
        {
            let account = account.clone();
            if !self.skip_initial_tracking {
                self.initial_accounts_state.insert(address, account.clone());
            }
            return Ok(self
                .current_accounts_state
                .entry(address)
                .or_insert(account));
        }

        // Store fallback.
        let state = self.store.get_account_state(address)?;
        let account = LevmAccount::from(state);
        if !self.skip_initial_tracking {
            self.initial_accounts_state.insert(address, account.clone());
        }
        Ok(self
            .current_accounts_state
            .entry(address)
            .or_insert(account))
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
        if self.skip_initial_tracking {
            return Ok(value);
        }
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
            let was_destroyed = new_state_account.status == AccountStatus::DestroyedModified;
            // Only emit removed_storage if the account actually had storage in the trie.
            // If it didn't (e.g. account was created within the batch), there's nothing to
            // remove, and emitting removed_storage=true would cause a spurious empty
            // account to be inserted into the state trie.
            let removed_storage = was_destroyed && initial_state_account.has_storage;

            // 2. Storage has been updated if the current value is different from the one before execution.
            let mut added_storage: FxHashMap<_, _> = Default::default();

            for (key, new_value) in &new_state_account.storage {
                let old_value = if !was_destroyed {
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
            let was_destroyed = new_state_account.status == AccountStatus::DestroyedModified;
            // Only emit removed_storage if the account actually had storage in the trie.
            // If it didn't (e.g. account was created within the batch), there's nothing to
            // remove, and emitting removed_storage=true would cause a spurious empty
            // account to be inserted into the state trie.
            let removed_storage = was_destroyed && initial_state_account.has_storage;

            // 2. Storage has been updated if the current value is different from the one before execution.
            let mut added_storage: FxHashMap<_, _> = Default::default();

            for (key, new_value) in &new_state_account.storage {
                let old_value = if !was_destroyed {
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
        // Backup must be taken before mark_modified flips `exists` to true.
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
        if increase.is_zero() {
            return Ok(());
        }
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
        if decrease.is_zero() {
            return Ok(());
        }
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

    /// SSTORE-specialized storage access path that returns current and original values together.
    /// This keeps the SSTORE hot path tighter by avoiding extra method-level plumbing.
    #[inline(always)]
    pub fn access_storage_slot_for_sstore(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<(U256, U256, bool), InternalError> {
        let storage_slot_was_cold = self.substate.add_accessed_slot(address, key);
        // SSTORE pre-image flows transitively through get_storage_value, which consults lazy_bal.
        let current_value = self.get_storage_value(address, key)?;
        let original_value = match self
            .storage_original_values
            .entry(address)
            .or_default()
            .entry(key)
        {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => *entry.insert(current_value),
        };
        Ok((current_value, original_value, storage_slot_was_cold))
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

        // Lazy-BAL hook: copy result out BEFORE taking &mut on current_accounts_state
        // so the immutable borrow of lazy_bal is released before the mutable reborrow.
        #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
        let bal_hit: Option<U256> = self.db.lazy_bal.as_ref().and_then(|cursor| {
            debug_assert!(
                cursor.bal_index >= 1,
                "LazyBalCursor bal_index must be >= 1"
            );
            let max_idx = cursor.bal_index.saturating_sub(1);
            let &acct_idx = cursor.index.addr_to_idx.get(&address)?;
            seed_one_storage_slot_from_bal(&cursor.bal, &cursor.index, acct_idx, key, max_idx)
        });
        #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
        if let Some(value) = bal_hit {
            let account = self
                .db
                .current_accounts_state
                .get_mut(&address)
                .ok_or(InternalError::AccountNotFound)?;
            account.storage.insert(key, value);
            return Ok(value);
        }

        let value = self.db.get_value_from_database(address, key)?;

        // Cache-fill only: this is a read-path miss, not a state mutation.
        let account = self
            .db
            .current_accounts_state
            .get_mut(&address)
            .ok_or(InternalError::AccountNotFound)?;
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
