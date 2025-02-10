pub mod backends;
pub mod db;
pub mod errors;
pub mod execution_db;
mod execution_result;
#[cfg(feature = "l2")]
mod mods;

use backends::EVM;
use db::EvmState;

use crate::backends::revm::*;
use ethrex_core::{
    types::{
        tx_fields::AccessList, AccountInfo, BlockHeader, ChainConfig, Fork, GenericTransaction,
        INITIAL_BASE_FEE,
    },
    Address, BigEndianHash, H256, U256,
};
use ethrex_storage::AccountUpdate;
use revm::{
    db::{states::bundle_state::BundleRetention, AccountState, AccountStatus},
    inspector_handle_register,
    primitives::{BlockEnv, TxEnv, B256},
    Evm,
};
// Rename imported types for clarity
use revm_primitives::AccessList as RevmAccessList;
// Export needed types
pub use errors::EvmError;
pub use execution_result::*;
pub use revm::primitives::{Address as RevmAddress, SpecId, U256 as RevmU256};

use std::sync::OnceLock;

// This global variable can be initialized by the ethrex cli.
// EVM_BACKEND.get_or_init(|| evm);
// Then, we can retrieve the evm with:
// EVM_BACKEND.get(); -> returns Option<EVM>
pub static EVM_BACKEND: OnceLock<EVM> = OnceLock::new();

// ================== Commonly used functions ======================

// TODO: IMPLEMENT FOR LEVM
// Executes a single GenericTransaction, doesn't commit the result or perform state transitions
pub fn simulate_tx_from_generic(
    tx: &GenericTransaction,
    header: &BlockHeader,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    let block_env = block_env(header);
    let tx_env = tx_env_from_generic(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    run_without_commit(tx_env, block_env, state, spec_id)
}

// TODO: IMPLEMENT FOR LEVM
/// Runs the transaction and returns the access list and estimated gas use (when running the tx with said access list)
pub fn create_access_list(
    tx: &GenericTransaction,
    header: &BlockHeader,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<(ExecutionResult, AccessList), EvmError> {
    let mut tx_env = tx_env_from_generic(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    let block_env = block_env(header);
    // Run tx with access list inspector

    let (execution_result, access_list) =
        create_access_list_inner(tx_env.clone(), block_env.clone(), state, spec_id)?;

    // Run the tx with the resulting access list and estimate its gas used
    let execution_result = if execution_result.is_success() {
        tx_env.access_list.extend(access_list.0.clone());

        run_without_commit(tx_env, block_env, state, spec_id)?
    } else {
        execution_result
    };
    let access_list: Vec<(Address, Vec<H256>)> = access_list
        .iter()
        .map(|item| {
            (
                Address::from_slice(item.address.0.as_slice()),
                item.storage_keys
                    .iter()
                    .map(|v| H256::from_slice(v.as_slice()))
                    .collect(),
            )
        })
        .collect();
    Ok((execution_result, access_list))
}

// TODO: IMPLEMENT FOR LEVM
/// Runs the transaction and returns the access list for it
fn create_access_list_inner(
    tx_env: TxEnv,
    block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<(ExecutionResult, RevmAccessList), EvmError> {
    let mut access_list_inspector = access_list_inspector(&tx_env, state, spec_id)?;
    #[allow(unused_mut)]
    let mut evm_builder = Evm::builder()
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        .with_spec_id(spec_id)
        .modify_cfg_env(|env| {
            env.disable_base_fee = true;
            env.disable_block_gas_limit = true
        })
        .with_external_context(&mut access_list_inspector);

    let tx_result = {
        match state {
            EvmState::Store(db) => {
                let mut evm = evm_builder
                    .with_db(db)
                    .append_handler_register(inspector_handle_register)
                    .build();
                evm.transact().map_err(EvmError::from)?
            }
            EvmState::Execution(db) => {
                let mut evm = evm_builder
                    .with_db(db)
                    .append_handler_register(inspector_handle_register)
                    .build();
                evm.transact().map_err(EvmError::from)?
            }
        }
    };

    let access_list = access_list_inspector.into_access_list();
    Ok((tx_result.result.into(), access_list))
}

/// Merges transitions stored when executing transactions and returns the resulting account updates
/// Doesn't update the DB
pub fn get_state_transitions(state: &mut EvmState) -> Vec<AccountUpdate> {
    match state {
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

/// Returns the spec id according to the block timestamp and the stored chain config
/// WARNING: Assumes at least Merge fork is active
pub fn spec_id(chain_config: &ChainConfig, block_timestamp: u64) -> SpecId {
    fork_to_spec_id(chain_config.get_fork(block_timestamp))
}

pub fn fork_to_spec_id(fork: Fork) -> SpecId {
    match fork {
        Fork::Frontier => SpecId::FRONTIER,
        Fork::FrontierThawing => SpecId::FRONTIER_THAWING,
        Fork::Homestead => SpecId::HOMESTEAD,
        Fork::DaoFork => SpecId::DAO_FORK,
        Fork::Tangerine => SpecId::TANGERINE,
        Fork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        Fork::Byzantium => SpecId::BYZANTIUM,
        Fork::Constantinople => SpecId::CONSTANTINOPLE,
        Fork::Petersburg => SpecId::PETERSBURG,
        Fork::Istanbul => SpecId::ISTANBUL,
        Fork::MuirGlacier => SpecId::MUIR_GLACIER,
        Fork::Berlin => SpecId::BERLIN,
        Fork::London => SpecId::LONDON,
        Fork::ArrowGlacier => SpecId::ARROW_GLACIER,
        Fork::GrayGlacier => SpecId::GRAY_GLACIER,
        Fork::Paris => SpecId::MERGE,
        Fork::Shanghai => SpecId::SHANGHAI,
        Fork::Cancun => SpecId::CANCUN,
        Fork::Prague => SpecId::PRAGUE,
        Fork::Osaka => SpecId::OSAKA,
    }
}
