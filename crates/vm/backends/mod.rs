mod constants;
pub mod levm;
pub mod revm_b;

use crate::{db::StoreWrapper, errors::EvmError, spec_id, EvmState, SpecId};
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{
    Block, BlockHeader, ChainConfig, Fork, Receipt, Transaction, Withdrawal,
};
use ethrex_common::{types::AccountInfo, Address, BigEndianHash, H256, U256};
use ethrex_levm::db::CacheDB;
use ethrex_storage::Store;
use ethrex_storage::{error::StoreError, AccountUpdate};
use levm::LEVM;
use revm_b::REVM;
use std::str::FromStr;
use std::sync::Arc;

use revm::db::states::bundle_state::BundleRetention;
use revm::db::{AccountState, AccountStatus};
use revm::primitives::B256;

#[derive(Debug, Default, Clone)]
pub enum EVM {
    #[default]
    REVM,
    LEVM,
}

impl FromStr for EVM {
    type Err = EvmError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "levm" => Ok(EVM::LEVM),
            "revm" => Ok(EVM::REVM),
            _ => Err(EvmError::InvalidEVM(s.to_string())),
        }
    }
}

pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Requests>,
    pub account_updates: Vec<AccountUpdate>,
}

impl EVM {
    /// Wraps [REVM::execute_block] and [LEVM::execute_block].
    /// The output is [BlockExecutionResult].
    pub fn execute_block(
        &self,
        block: &Block,
        storage: Store,
    ) -> Result<BlockExecutionResult, EvmError> {
        match self {
            EVM::REVM => REVM::execute_block(block, storage),
            EVM::LEVM => LEVM::execute_block(block, storage),
        }
    }

    /// Wraps [REVM::execute_tx] and [LEVM::execute_tx].
    /// The output is `(Receipt, u64)` == (transaction_receipt, gas_used).
    pub fn execute_tx(
        &self,
        state: &mut EvmState,
        tx: &Transaction,
        block_header: &BlockHeader,
        block_cache: &mut CacheDB,
        chain_config: &ChainConfig,
        remaining_gas: &mut u64,
    ) -> Result<(Receipt, u64), EvmError> {
        match self {
            EVM::REVM => {
                let execution_result = REVM::execute_tx(
                    tx,
                    block_header,
                    state,
                    spec_id(chain_config, block_header.timestamp),
                )?;

                *remaining_gas = remaining_gas.saturating_sub(execution_result.gas_used());

                let receipt = Receipt::new(
                    tx.tx_type(),
                    execution_result.is_success(),
                    block_header.gas_limit - *remaining_gas,
                    execution_result.logs(),
                );

                Ok((receipt, execution_result.gas_used()))
            }
            EVM::LEVM => {
                let store_wrapper = Arc::new(StoreWrapper {
                    store: state.database().unwrap().clone(),
                    block_hash: block_header.parent_hash,
                });

                let execution_report = LEVM::execute_tx(
                    tx,
                    block_header,
                    store_wrapper.clone(),
                    block_cache.clone(),
                    chain_config,
                )?;

                *remaining_gas = remaining_gas.saturating_sub(execution_report.gas_used);

                let mut new_state = execution_report.new_state.clone();

                // Now original_value is going to be the same as the current_value, for the next transaction.
                // It should have only one value but it is convenient to keep on using our CacheDB structure
                for account in new_state.values_mut() {
                    for storage_slot in account.storage.values_mut() {
                        storage_slot.original_value = storage_slot.current_value;
                    }
                }
                block_cache.extend(new_state);

                let receipt = Receipt::new(
                    tx.tx_type(),
                    execution_report.is_success(),
                    block_header.gas_limit - *remaining_gas,
                    execution_report.logs.clone(),
                );
                Ok((receipt, execution_report.gas_used))
            }
        }
    }

    /// Wraps [REVM::beacon_root_contract_call], [REVM::process_block_hash_history]
    /// and [LEVM::beacon_root_contract_call], [LEVM::process_block_hash_history].
    /// This function is used to run/apply all the system contracts to the state.
    pub fn apply_system_calls(
        &self,
        state: &mut EvmState,
        block_header: &BlockHeader,
        block_cache: &mut CacheDB,
        chain_config: &ChainConfig,
    ) -> Result<(), EvmError> {
        match self {
            EVM::REVM => {
                let spec_id = spec_id(chain_config, block_header.timestamp);
                if block_header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
                    REVM::beacon_root_contract_call(block_header, state)?;
                }

                if spec_id >= SpecId::PRAGUE {
                    REVM::process_block_hash_history(block_header, state)?;
                }
                Ok(())
            }
            EVM::LEVM => {
                let fork = chain_config.fork(block_header.timestamp);
                let mut new_state = CacheDB::new();
                let store = state.database().unwrap().clone(); //TODO: Change this in apply_system_calls maybe...

                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    LEVM::beacon_root_contract_call(block_header, &store, &mut new_state)?;
                }

                if fork >= Fork::Prague {
                    LEVM::process_block_hash_history(block_header, &store, &mut new_state)?;
                }

                // Now original_value is going to be the same as the current_value, for the next transaction.
                // It should have only one value but it is convenient to keep on using our CacheDB structure
                for account in new_state.values_mut() {
                    for storage_slot in account.storage.values_mut() {
                        storage_slot.original_value = storage_slot.current_value;
                    }
                }

                block_cache.extend(new_state);
                Ok(())
            }
        }
    }

    /// Wraps the [REVM::get_state_transitions] and [LEVM::get_state_transitions].
    /// The output is `Vec<AccountUpdate>`.
    /// WARNING:
    /// [REVM::get_state_transitions] gathers the information from the DB, the functionality of this function
    /// is used in [LEVM::execute_block].
    /// [LEVM::get_state_transitions] gathers the information from a [CacheDB].
    ///
    /// They may have the same name, but they serve for different purposes.
    pub fn get_state_transitions(
        &self,
        state: &mut EvmState,
        parent_hash: H256,
        block_cache: &CacheDB,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        match self {
            EVM::REVM => REVM::get_state_transitions(state),
            EVM::LEVM => LEVM::get_state_transitions(
                None,
                state.database().unwrap(),
                parent_hash,
                block_cache,
            ),
        }
    }

    /// Wraps the [REVM::process_withdrawals] and [LEVM::process_withdrawals].
    /// Applies the withdrawals to the state or the block_chache if using [LEVM].
    pub fn process_withdrawals(
        &self,
        withdrawals: &[Withdrawal],
        state: &mut EvmState,
        block_header: &BlockHeader,
        block_cache: &mut CacheDB,
    ) -> Result<(), StoreError> {
        match self {
            EVM::REVM => REVM::process_withdrawals(state, withdrawals),
            EVM::LEVM => {
                let parent_hash = block_header.parent_hash;
                let mut new_state = CacheDB::new();
                LEVM::process_withdrawals(
                    &mut new_state,
                    withdrawals,
                    state.database(),
                    parent_hash,
                )?;
                block_cache.extend(new_state);
                Ok(())
            }
        }
    }

    pub fn extract_requests(
        &self,
        receipts: &[Receipt],
        state: &mut EvmState,
        header: &BlockHeader,
        cache: &mut CacheDB,
    ) -> Result<Vec<Requests>, EvmError> {
        match self {
            EVM::LEVM => {
                levm::extract_all_requests_levm(receipts, state.database().unwrap(), header, cache)
            }
            EVM::REVM => revm_b::extract_all_requests(receipts, state, header),
        }
    }
}

/// Gets the state_transitions == [AccountUpdate] from the [EvmState].
/// This function is primarily used in [LEVM::execute_block] and [REVM::execute_block].
pub fn get_state_transitions(initial_state: &mut EvmState) -> Vec<ethrex_storage::AccountUpdate> {
    match initial_state {
        EvmState::Store(db) => {
            db.merge_transitions(BundleRetention::PlainState);
            let bundle = db.take_bundle();

            // Update accounts
            let mut account_updates = Vec::new();
            for (address, account) in bundle.state() {
                if account.status.is_not_modified() {
                    continue;
                }
                let address = Address::from_slice(address.0.as_slice());
                // Remove account from DB if destroyed (Process DestroyedChanged as changed account)
                if matches!(
                    account.status,
                    AccountStatus::Destroyed | AccountStatus::DestroyedAgain
                ) {
                    account_updates.push(AccountUpdate::removed(address));
                    continue;
                }

                // If account is empty, do not add to the database
                if account
                    .account_info()
                    .is_some_and(|acc_info| acc_info.is_empty())
                {
                    continue;
                }

                // Apply account changes to DB
                let mut account_update = AccountUpdate::new(address);
                // If the account was changed then both original and current info will be present in the bundle account
                if account.is_info_changed() {
                    // Update account info in DB
                    if let Some(new_acc_info) = account.account_info() {
                        let code_hash = H256::from_slice(new_acc_info.code_hash.as_slice());
                        let account_info = AccountInfo {
                            code_hash,
                            balance: U256::from_little_endian(new_acc_info.balance.as_le_slice()),
                            nonce: new_acc_info.nonce,
                        };
                        account_update.info = Some(account_info);
                        if account.is_contract_changed() {
                            // Update code in db
                            if let Some(code) = new_acc_info.code {
                                account_update.code = Some(code.original_bytes().clone().0);
                            }
                        }
                    }
                }
                // Update account storage in DB
                for (key, slot) in account.storage.iter() {
                    if slot.is_changed() {
                        // TODO check if we need to remove the value from our db when value is zero
                        // if slot.present_value().is_zero() {
                        //     account_update.removed_keys.push(H256::from_uint(&U256::from_little_endian(key.as_le_slice())))
                        // }
                        account_update.added_storage.insert(
                            H256::from_uint(&U256::from_little_endian(key.as_le_slice())),
                            U256::from_little_endian(slot.present_value().as_le_slice()),
                        );
                    }
                }
                account_updates.push(account_update)
            }
            account_updates
        }
        EvmState::Execution(db) => {
            // Update accounts
            let mut account_updates = Vec::new();
            for (revm_address, account) in &db.accounts {
                if account.account_state == AccountState::None {
                    // EVM didn't interact with this account
                    continue;
                }

                let address = Address::from_slice(revm_address.0.as_slice());
                // Remove account from DB if destroyed
                if account.account_state == AccountState::NotExisting {
                    account_updates.push(AccountUpdate::removed(address));
                    continue;
                }

                // If account is empty, do not add to the database
                if account.info().is_some_and(|acc_info| acc_info.is_empty()) {
                    continue;
                }

                // Apply account changes to DB
                let mut account_update = AccountUpdate::new(address);
                // Update account info in DB
                if let Some(new_acc_info) = account.info() {
                    // If code changed, update
                    if matches!(db.db.accounts.get(&address), Some(account) if B256::from(account.code_hash.0) != new_acc_info.code_hash)
                    {
                        account_update.code = new_acc_info
                            .code
                            .map(|code| bytes::Bytes::copy_from_slice(code.bytes_slice()));
                    }

                    let account_info = AccountInfo {
                        code_hash: H256::from_slice(new_acc_info.code_hash.as_slice()),
                        balance: U256::from_little_endian(new_acc_info.balance.as_le_slice()),
                        nonce: new_acc_info.nonce,
                    };
                    account_update.info = Some(account_info);
                }
                // Update account storage in DB
                for (key, slot) in account.storage.iter() {
                    // TODO check if we need to remove the value from our db when value is zero
                    // if slot.present_value().is_zero() {
                    //     account_update.removed_keys.push(H256::from_uint(&U256::from_little_endian(key.as_le_slice())))
                    // }
                    account_update.added_storage.insert(
                        H256::from_uint(&U256::from_little_endian(key.as_le_slice())),
                        U256::from_little_endian(slot.as_le_slice()),
                    );
                }
                account_updates.push(account_update)
            }
            account_updates
        }
    }
}
