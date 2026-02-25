//! LevmHost — revm Host implementation backed by LEVM state.
//!
//! This module bridges LEVM's execution state to the revm `Host` trait that
//! revmc's JIT-compiled code expects. Each Host method delegates to the
//! corresponding LEVM `GeneralizedDatabase` or `Substate` operation.
//!
//! # Phase 3 Scope
//!
//! For pure-computation bytecodes (Fibonacci), only the block/tx/config getters
//! and basic account loading are exercised. Full SSTORE/SLOAD/CALL support
//! is wired but lightly tested until Phase 4.

use std::borrow::Cow;

use revm_context_interface::{
    cfg::GasParams,
    context::{SStoreResult, SelfDestructResult, StateLoad},
    host::LoadError,
    journaled_state::AccountInfoLoad,
};
use revm_interpreter::Host;
use revm_primitives::{Address as RevmAddress, B256, Log as RevmLog, U256 as RevmU256};
use revm_state::AccountInfo as RevmAccountInfo;

use crate::adapter::{
    fork_to_spec_id, levm_address_to_revm, levm_h256_to_revm, levm_u256_to_revm,
    revm_address_to_levm, revm_u256_to_levm,
};
use ethrex_levm::account::AccountStatus;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::Environment;
use ethrex_levm::errors::InternalError;
use ethrex_levm::vm::Substate;

/// revm Host implementation backed by LEVM state.
///
/// Holds mutable references to the LEVM database, substate, and environment
/// so JIT-compiled code can interact with the EVM world state.
pub struct LevmHost<'a> {
    pub db: &'a mut GeneralizedDatabase,
    pub substate: &'a mut Substate,
    pub env: &'a Environment,
    pub address: ethrex_common::Address,
    gas_params: GasParams,
    /// Original storage values before the transaction (for SSTORE gas calculation).
    pub storage_original_values: &'a mut ethrex_levm::jit::dispatch::StorageOriginalValues,
    /// Journal of storage writes: (address, key, previous_value).
    /// Used to rollback storage on REVERT. Each entry records the value
    /// that was present before the SSTORE, so reverting replays in reverse.
    pub(crate) storage_journal: Vec<(
        ethrex_common::Address,
        ethrex_common::H256,
        ethrex_common::U256,
    )>,
}

impl<'a> LevmHost<'a> {
    pub fn new(
        db: &'a mut GeneralizedDatabase,
        substate: &'a mut Substate,
        env: &'a Environment,
        address: ethrex_common::Address,
        storage_original_values: &'a mut ethrex_levm::jit::dispatch::StorageOriginalValues,
    ) -> Self {
        let spec_id = fork_to_spec_id(env.config.fork);
        let gas_params = GasParams::new_spec(spec_id);
        Self {
            db,
            substate,
            env,
            address,
            gas_params,
            storage_original_values,
            storage_journal: Vec::new(),
        }
    }
}

impl Host for LevmHost<'_> {
    // === Block getters ===

    fn basefee(&self) -> RevmU256 {
        levm_u256_to_revm(&self.env.base_fee_per_gas)
    }

    fn blob_gasprice(&self) -> RevmU256 {
        levm_u256_to_revm(&self.env.base_blob_fee_per_gas)
    }

    fn gas_limit(&self) -> RevmU256 {
        RevmU256::from(self.env.block_gas_limit)
    }

    fn difficulty(&self) -> RevmU256 {
        levm_u256_to_revm(&self.env.difficulty)
    }

    fn prevrandao(&self) -> Option<RevmU256> {
        self.env.prev_randao.map(|h| {
            let b256 = levm_h256_to_revm(&h);
            RevmU256::from_be_bytes(b256.0)
        })
    }

    fn block_number(&self) -> RevmU256 {
        levm_u256_to_revm(&self.env.block_number)
    }

    fn timestamp(&self) -> RevmU256 {
        levm_u256_to_revm(&self.env.timestamp)
    }

    fn beneficiary(&self) -> RevmAddress {
        levm_address_to_revm(&self.env.coinbase)
    }

    fn chain_id(&self) -> RevmU256 {
        levm_u256_to_revm(&self.env.chain_id)
    }

    // === Transaction getters ===

    fn effective_gas_price(&self) -> RevmU256 {
        levm_u256_to_revm(&self.env.gas_price)
    }

    fn caller(&self) -> RevmAddress {
        levm_address_to_revm(&self.env.origin)
    }

    fn blob_hash(&self, number: usize) -> Option<RevmU256> {
        self.env.tx_blob_hashes.get(number).map(|h| {
            let b256 = levm_h256_to_revm(h);
            RevmU256::from_be_bytes(b256.0)
        })
    }

    // === Config ===

    fn max_initcode_size(&self) -> usize {
        // EIP-3860: 2 * MAX_CODE_SIZE = 2 * 24576 = 49152
        49152
    }

    fn gas_params(&self) -> &GasParams {
        &self.gas_params
    }

    // === Database ===

    fn block_hash(&mut self, number: u64) -> Option<B256> {
        self.db
            .store
            .get_block_hash(number)
            .ok()
            .map(|h| levm_h256_to_revm(&h))
    }

    // === Journal (state mutation) ===

    fn load_account_info_skip_cold_load(
        &mut self,
        address: RevmAddress,
        load_code: bool,
        _skip_cold_load: bool,
    ) -> Result<AccountInfoLoad<'_>, LoadError> {
        let levm_addr = revm_address_to_levm(&address);
        let account = self
            .db
            .get_account(levm_addr)
            .map_err(|_| LoadError::DBError)?;

        // Extract all fields from account before dropping the borrow,
        // so we can call self.db.get_code() below without a double borrow.
        let balance = levm_u256_to_revm(&account.info.balance);
        let nonce = account.info.nonce;
        let levm_code_hash = account.info.code_hash;
        let is_empty = account.info.balance.is_zero()
            && nonce == 0
            && levm_code_hash == *ethrex_common::constants::EMPTY_KECCACK_HASH;
        let code_hash = levm_h256_to_revm(&levm_code_hash);

        // Now account borrow is dropped, safe to borrow self.db again.
        let code = if load_code {
            let code_ref = self
                .db
                .get_code(levm_code_hash)
                .map_err(|_| LoadError::DBError)?;
            Some(revm_bytecode::Bytecode::new_raw(revm_primitives::Bytes(
                code_ref.bytecode.clone(),
            )))
        } else {
            None
        };

        let info = RevmAccountInfo {
            balance,
            nonce,
            code_hash,
            account_id: None,
            code,
        };

        // Mark address as accessed for EIP-2929 warm/cold tracking
        let is_cold = !self.substate.add_accessed_address(levm_addr);

        Ok(AccountInfoLoad {
            account: Cow::Owned(info),
            is_cold,
            is_empty,
        })
    }

    // TODO(JIT): EIP-7928 BAL recording not implemented for JIT execution path.
    // LEVM's get_storage_value records BAL entries via bal_recorder. The JIT path
    // bypasses this. Add BAL recording when JIT moves beyond PoC phase.
    fn sload_skip_cold_load(
        &mut self,
        address: RevmAddress,
        key: RevmU256,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<RevmU256>, LoadError> {
        let levm_addr = revm_address_to_levm(&address);
        let levm_key = ethrex_common::H256::from(revm_u256_to_levm(&key).to_big_endian());

        let value =
            jit_get_storage_value(self.db, levm_addr, levm_key).map_err(|_| LoadError::DBError)?;

        // EIP-2929: track cold/warm storage slot access
        let is_cold = !self.substate.add_accessed_slot(levm_addr, levm_key);

        Ok(StateLoad::new(levm_u256_to_revm(&value), is_cold))
    }

    // TODO(JIT): EIP-7928 BAL recording not implemented for JIT execution path.
    // LEVM's update_account_storage records BAL entries via bal_recorder. The JIT
    // path bypasses this. Add BAL recording when JIT moves beyond PoC phase.
    fn sstore_skip_cold_load(
        &mut self,
        address: RevmAddress,
        key: RevmU256,
        value: RevmU256,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SStoreResult>, LoadError> {
        let levm_addr = revm_address_to_levm(&address);
        let levm_key_u256 = revm_u256_to_levm(&key);
        let levm_key = ethrex_common::H256::from(levm_key_u256.to_big_endian());
        let levm_value = revm_u256_to_levm(&value);

        // EIP-2929: track cold/warm storage slot access
        let is_cold = !self.substate.add_accessed_slot(levm_addr, levm_key);

        // Get current (present) value before write
        let present =
            jit_get_storage_value(self.db, levm_addr, levm_key).map_err(|_| LoadError::DBError)?;

        // Get or cache the pre-tx original value for SSTORE gas calculation
        let cache_key = (levm_addr, levm_key);
        let original = *self
            .storage_original_values
            .entry(cache_key)
            .or_insert(present);

        // Record pre-write value for rollback on REVERT
        self.storage_journal.push((levm_addr, levm_key, present));

        // Write new value directly into the account's cached storage
        jit_update_account_storage(self.db, levm_addr, levm_key, levm_value)
            .map_err(|_| LoadError::DBError)?;

        Ok(StateLoad::new(
            SStoreResult {
                original_value: levm_u256_to_revm(&original),
                present_value: levm_u256_to_revm(&present),
                new_value: value,
            },
            is_cold,
        ))
    }

    fn tload(&mut self, _address: RevmAddress, key: RevmU256) -> RevmU256 {
        let levm_addr = revm_address_to_levm(&_address);
        let levm_key = revm_u256_to_levm(&key);
        let value = self.substate.get_transient(&levm_addr, &levm_key);
        levm_u256_to_revm(&value)
    }

    fn tstore(&mut self, _address: RevmAddress, key: RevmU256, value: RevmU256) {
        let levm_addr = revm_address_to_levm(&_address);
        let levm_key = revm_u256_to_levm(&key);
        let levm_value = revm_u256_to_levm(&value);
        self.substate
            .set_transient(&levm_addr, &levm_key, levm_value);
    }

    fn log(&mut self, log: RevmLog) {
        let levm_address = revm_address_to_levm(&log.address);
        let topics: Vec<ethrex_common::H256> = log
            .data
            .topics()
            .iter()
            .map(|t| ethrex_common::H256::from_slice(t.as_slice()))
            .collect();
        let data = log.data.data.to_vec();

        let levm_log = ethrex_common::types::Log {
            address: levm_address,
            topics,
            data: bytes::Bytes::from(data),
        };
        self.substate.add_log(levm_log);
    }

    fn selfdestruct(
        &mut self,
        address: RevmAddress,
        target: RevmAddress,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SelfDestructResult>, LoadError> {
        let levm_addr = revm_address_to_levm(&address);
        let levm_target = revm_address_to_levm(&target);

        let previously_destroyed = self.substate.add_selfdestruct(levm_addr);

        // Check if the self-destructing account has a non-zero balance
        let had_value = self
            .db
            .get_account(levm_addr)
            .map(|a| !a.info.balance.is_zero())
            .unwrap_or(false);

        // Check if the target account exists (non-empty per EIP-161)
        let target_exists = self
            .db
            .get_account(levm_target)
            .map(|a| !a.info.is_empty())
            .unwrap_or(false);

        // EIP-2929: track cold/warm access for the target address
        let is_cold = !self.substate.add_accessed_address(levm_target);

        Ok(StateLoad::new(
            SelfDestructResult {
                had_value,
                target_exists,
                previously_destroyed,
            },
            is_cold,
        ))
    }
}

/// Read a storage value from the generalized database, replicating the logic
/// of `VM::get_storage_value` without needing access to the call frame backups.
///
/// 1. Check the current accounts state cache.
/// 2. If account was destroyed-and-modified, return zero (storage is invalid).
/// 3. Fall back to the underlying `Database::get_storage_value`.
/// 4. Cache the result in both `current_accounts_state` and `initial_accounts_state`.
///
// TODO(JIT): EIP-7928 BAL recording not implemented for JIT execution path.
// LEVM's get_storage_value records BAL entries via bal_recorder. The JIT path
// bypasses this. Add BAL recording when JIT moves beyond PoC phase.
fn jit_get_storage_value(
    db: &mut GeneralizedDatabase,
    address: ethrex_common::Address,
    key: ethrex_common::H256,
) -> Result<ethrex_common::U256, InternalError> {
    // Ensure the account is loaded into the cache first.
    let _ = db.get_account(address)?;

    if let Some(account) = db.current_accounts_state.get(&address) {
        if let Some(value) = account.storage.get(&key) {
            return Ok(*value);
        }
        // If the account was destroyed and then re-created, DB storage is stale.
        if account.status == AccountStatus::DestroyedModified {
            return Ok(ethrex_common::U256::zero());
        }
    } else {
        return Err(InternalError::AccountNotFound);
    }

    // Fall back to the persistent store.
    let value = db.store.get_storage_value(address, key)?;

    // Cache in initial_accounts_state (for state-diff calculation).
    if let Some(account) = db.initial_accounts_state.get_mut(&address) {
        account.storage.insert(key, value);
    }

    // Cache in current_accounts_state so subsequent reads are fast.
    if let Some(account) = db.current_accounts_state.get_mut(&address) {
        account.storage.insert(key, value);
    }

    Ok(value)
}

/// Write a storage value into the generalized database, replicating the
/// essential logic of `VM::update_account_storage` without call frame backups.
///
// TODO(JIT): EIP-7928 BAL recording not implemented for JIT execution path.
// LEVM's update_account_storage records BAL entries via bal_recorder. The JIT
// path bypasses this. Add BAL recording when JIT moves beyond PoC phase.
pub(crate) fn jit_update_account_storage(
    db: &mut GeneralizedDatabase,
    address: ethrex_common::Address,
    key: ethrex_common::H256,
    new_value: ethrex_common::U256,
) -> Result<(), InternalError> {
    let account = db.get_account_mut(address)?;
    account.storage.insert(key, new_value);
    Ok(())
}
