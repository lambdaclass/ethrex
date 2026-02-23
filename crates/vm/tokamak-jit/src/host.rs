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
use revm_primitives::{Address as RevmAddress, B256, Log as RevmLog, SpecId, U256 as RevmU256};
use revm_state::AccountInfo as RevmAccountInfo;

use crate::adapter::{
    levm_address_to_revm, levm_h256_to_revm, levm_u256_to_revm, revm_address_to_levm,
    revm_u256_to_levm,
};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::Environment;
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
}

impl<'a> LevmHost<'a> {
    pub fn new(
        db: &'a mut GeneralizedDatabase,
        substate: &'a mut Substate,
        env: &'a Environment,
        address: ethrex_common::Address,
    ) -> Self {
        let gas_params = GasParams::new_spec(SpecId::CANCUN);
        Self {
            db,
            substate,
            env,
            address,
            gas_params,
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
        let account = self.db.get_account(levm_addr).map_err(|_| LoadError::DBError)?;

        let balance = levm_u256_to_revm(&account.info.balance);
        let code_hash = levm_h256_to_revm(&account.info.code_hash);

        let code = if load_code {
            let code_ref = self
                .db
                .get_code(account.info.code_hash)
                .map_err(|_| LoadError::DBError)?;
            Some(revm_bytecode::Bytecode::new_raw(
                code_ref.bytecode.clone(),
            ))
        } else {
            None
        };

        let is_empty = account.info.balance.is_zero()
            && account.info.nonce == 0
            && account.info.code_hash == ethrex_common::constants::EMPTY_KECCACK_HASH;

        let info = RevmAccountInfo {
            balance,
            nonce: account.info.nonce,
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

    fn sload_skip_cold_load(
        &mut self,
        address: RevmAddress,
        key: RevmU256,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<RevmU256>, LoadError> {
        let levm_addr = revm_address_to_levm(&address);
        let levm_key = ethrex_common::H256::from(revm_u256_to_levm(&key).to_big_endian());

        let value = self
            .db
            .get_storage_value(levm_addr, levm_key)
            .map_err(|_| LoadError::DBError)?;

        Ok(StateLoad::new(levm_u256_to_revm(&value), false))
    }

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

        // Get current value before write
        let current = self
            .db
            .get_storage_value(levm_addr, levm_key)
            .map_err(|_| LoadError::DBError)?;

        // Write new value
        self.db
            .update_account_storage(levm_addr, levm_key, levm_key_u256, levm_value, current)
            .map_err(|_| LoadError::DBError)?;

        Ok(StateLoad::new(
            SStoreResult {
                original_value: levm_u256_to_revm(&current),
                present_value: levm_u256_to_revm(&current),
                new_value: value,
            },
            false,
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
        _target: RevmAddress,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SelfDestructResult>, LoadError> {
        let levm_addr = revm_address_to_levm(&address);
        let previously_destroyed = self.substate.add_selfdestruct(levm_addr);

        Ok(StateLoad::new(
            SelfDestructResult {
                had_value: false,
                target_exists: true,
                previously_destroyed,
            },
            false,
        ))
    }
}
