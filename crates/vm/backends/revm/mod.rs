pub mod db;
pub mod execution_db;
pub mod helpers;
#[cfg(feature = "l2")]
mod mods;

use super::BlockExecutionResult;
use crate::constants::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::errors::EvmError;
use crate::execution_result::ExecutionResult;
use crate::helpers::spec_id;
use db::EvmState;
use ethrex_common::types::AccountInfo;
use ethrex_common::{BigEndianHash, H256, U256};
use ethrex_storage::{error::StoreError, AccountUpdate};

use revm::db::states::bundle_state::BundleRetention;
use revm::db::AccountStatus;
use revm::{
    db::AccountState as RevmAccountState,
    inspectors::TracerEip3155,
    primitives::{BlobExcessGasAndPrice, BlockEnv, TxEnv, B256},
    DatabaseCommit, Evm,
};
use revm_inspectors::access_list::AccessListInspector;
// Rename imported types for clarity
use ethrex_common::{
    types::{
        requests::Requests, Block, BlockHeader, GenericTransaction, Receipt, Transaction, TxKind,
        Withdrawal, GWEI_TO_WEI, INITIAL_BASE_FEE,
    },
    Address,
};
use revm_primitives::Bytes;
use revm_primitives::{
    ruint::Uint, AccessList as RevmAccessList, AccessListItem, Address as RevmAddress,
    Authorization as RevmAuthorization, FixedBytes, SignedAuthorization, SpecId,
    TxKind as RevmTxKind, U256 as RevmU256,
};
use std::cmp::min;

#[derive(Debug)]
pub struct REVM;

/// The struct implements the following functions:
/// [REVM::execute_block]
/// [REVM::execute_tx]
/// [REVM::get_state_transitions]
/// [REVM::process_withdrawals]
impl REVM {
    pub fn execute_block(
        block: &Block,
        state: &mut EvmState,
    ) -> Result<BlockExecutionResult, EvmError> {
        let block_header = &block.header;
        let spec_id: SpecId = spec_id(&state.chain_config()?, block_header.timestamp);
        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                if block_header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
                    Self::beacon_root_contract_call(block_header, state)?;
                }

                //eip 2935: stores parent block hash in system contract
                if spec_id >= SpecId::PRAGUE {
                    Self::process_block_hash_history(block_header, state)?;
                }
            }
        }
        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for tx in block.body.transactions.iter() {
            let result = Self::execute_tx(tx, block_header, state, spec_id, tx.sender())?;
            cumulative_gas_used += result.gas_used();
            let receipt = Receipt::new(
                tx.tx_type(),
                result.is_success(),
                cumulative_gas_used,
                result.logs(),
            );
            receipts.push(receipt);
        }

        if let Some(withdrawals) = &block.body.withdrawals {
            Self::process_withdrawals(state, withdrawals)?;
        }

        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                let requests = extract_all_requests(&receipts, state, block_header)?;
            } else {
                let requests = Default::default();
            }
        }

        let account_updates = Self::get_state_transitions(state);

        Ok(BlockExecutionResult {
            receipts,
            requests,
            account_updates,
        })
    }

    pub fn execute_tx(
        tx: &Transaction,
        header: &BlockHeader,
        state: &mut EvmState,
        spec_id: SpecId,
        sender: Address,
    ) -> Result<ExecutionResult, EvmError> {
        let block_env = block_env(header, spec_id);
        let tx_env = tx_env(tx, sender);
        run_evm(tx_env, block_env, state, spec_id)
    }

    pub fn process_withdrawals(
        initial_state: &mut EvmState,
        withdrawals: &[Withdrawal],
    ) -> Result<(), StoreError> {
        //balance_increments is a vector of tuples (Address, increment as u128)
        let balance_increments = withdrawals
            .iter()
            .filter(|withdrawal| withdrawal.amount > 0)
            .map(|withdrawal| {
                (
                    RevmAddress::from_slice(withdrawal.address.as_bytes()),
                    (withdrawal.amount as u128 * GWEI_TO_WEI as u128),
                )
            })
            .collect::<Vec<_>>();
        match initial_state {
            EvmState::Store(db) => {
                db.increment_balances(balance_increments)?;
            }
            EvmState::Execution(db) => {
                for (address, balance) in balance_increments {
                    if balance == 0 {
                        continue;
                    }

                    let account = db
                        .load_account(address)
                        .map_err(|err| StoreError::Custom(format!("revm CacheDB error: {err}")))?;

                    account.info.balance += RevmU256::from(balance);
                    if account.account_state == RevmAccountState::None {
                        account.account_state = RevmAccountState::Touched;
                    }
                }
            }
        }
        Ok(())
    }

    // SYSTEM CONTRACTS
    pub fn beacon_root_contract_call(
        block_header: &BlockHeader,
        state: &mut EvmState,
    ) -> Result<(), EvmError> {
        let beacon_root = match block_header.parent_beacon_block_root {
            None => {
                return Err(EvmError::Header(
                    "parent_beacon_block_root field is missing".to_string(),
                ))
            }
            Some(beacon_root) => beacon_root,
        };

        generic_system_contract_revm(
            block_header,
            Bytes::copy_from_slice(beacon_root.as_bytes()),
            state,
            *BEACON_ROOTS_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    pub fn process_block_hash_history(
        block_header: &BlockHeader,
        state: &mut EvmState,
    ) -> Result<(), EvmError> {
        generic_system_contract_revm(
            block_header,
            Bytes::copy_from_slice(block_header.parent_hash.as_bytes()),
            state,
            *HISTORY_STORAGE_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    pub(crate) fn read_withdrawal_requests(
        block_header: &BlockHeader,
        state: &mut EvmState,
    ) -> Option<Vec<u8>> {
        let tx_result = generic_system_contract_revm(
            block_header,
            Bytes::new(),
            state,
            *WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )
        .ok()?;

        if tx_result.is_success() {
            Some(tx_result.output().into())
        } else {
            None
        }
    }
    pub(crate) fn dequeue_consolidation_requests(
        block_header: &BlockHeader,
        state: &mut EvmState,
    ) -> Option<Vec<u8>> {
        let tx_result = generic_system_contract_revm(
            block_header,
            Bytes::new(),
            state,
            *CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )
        .ok()?;

        if tx_result.is_success() {
            Some(tx_result.output().into())
        } else {
            None
        }
    }

    /// Gets the state_transitions == [AccountUpdate] from the [EvmState].
    pub fn get_state_transitions(
        initial_state: &mut EvmState,
    ) -> Vec<ethrex_storage::AccountUpdate> {
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
                                balance: U256::from_little_endian(
                                    new_acc_info.balance.as_le_slice(),
                                ),
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
                    if account.account_state == RevmAccountState::None {
                        // EVM didn't interact with this account
                        continue;
                    }

                    let address = Address::from_slice(revm_address.0.as_slice());
                    // Remove account from DB if destroyed
                    if account.account_state == RevmAccountState::NotExisting {
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
}

/// Runs the transaction and returns the result, but does not commit it.
pub fn run_without_commit(
    tx_env: TxEnv,
    mut block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    adjust_disabled_base_fee(
        &mut block_env,
        tx_env.gas_price,
        tx_env.max_fee_per_blob_gas,
    );
    let chain_config = state.chain_config()?;
    #[allow(unused_mut)]
    let mut evm_builder = Evm::builder()
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        .with_spec_id(spec_id)
        .modify_cfg_env(|env| {
            env.disable_base_fee = true;
            env.disable_block_gas_limit = true;
            env.chain_id = chain_config.chain_id;
        });
    let tx_result = match state {
        EvmState::Store(db) => {
            let mut evm = evm_builder.with_db(db).build();
            evm.transact().map_err(EvmError::from)?
        }
        EvmState::Execution(db) => {
            let mut evm = evm_builder.with_db(db).build();
            evm.transact().map_err(EvmError::from)?
        }
    };
    Ok(tx_result.result.into())
}

/// Runs EVM, doesn't perform state transitions, but stores them
fn run_evm(
    tx_env: TxEnv,
    block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    let tx_result = {
        let chain_spec = state.chain_config()?;
        #[allow(unused_mut)]
        let mut evm_builder = Evm::builder()
            .with_block_env(block_env)
            .with_tx_env(tx_env)
            .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain_id)
            .with_spec_id(spec_id)
            .with_external_context(
                TracerEip3155::new(Box::new(std::io::stderr())).without_summary(),
            );
        cfg_if::cfg_if! {
            if #[cfg(feature = "l2")] {
                use revm::{Handler, primitives::{CancunSpec, HandlerCfg}};
                use std::sync::Arc;

                evm_builder = evm_builder.with_handler({
                    let mut evm_handler = Handler::new(HandlerCfg::new(SpecId::LATEST));
                    evm_handler.pre_execution.deduct_caller = Arc::new(mods::deduct_caller::<CancunSpec, _, _>);
                    evm_handler.validation.tx_against_state = Arc::new(mods::validate_tx_against_state::<CancunSpec, _, _>);
                    // TODO: Override `end` function. We should deposit even if we revert.
                    // evm_handler.pre_execution.end
                    evm_handler
                });
            }
        }

        match state {
            EvmState::Store(db) => {
                let mut evm = evm_builder.with_db(db).build();
                evm.transact_commit().map_err(EvmError::from)?
            }
            EvmState::Execution(db) => {
                let mut evm = evm_builder.with_db(db).build();
                evm.transact_commit().map_err(EvmError::from)?
            }
        }
    };
    Ok(tx_result.into())
}

/// Processes a block's withdrawals, updating the account balances in the state
pub fn process_withdrawals(
    state: &mut EvmState,
    withdrawals: &[Withdrawal],
) -> Result<(), StoreError> {
    //balance_increments is a vector of tuples (Address, increment as u128)
    let balance_increments = withdrawals
        .iter()
        .filter(|withdrawal| withdrawal.amount > 0)
        .map(|withdrawal| {
            (
                RevmAddress::from_slice(withdrawal.address.as_bytes()),
                (withdrawal.amount as u128 * GWEI_TO_WEI as u128),
            )
        })
        .collect::<Vec<_>>();
    match state {
        EvmState::Store(db) => {
            db.increment_balances(balance_increments)?;
        }
        EvmState::Execution(db) => {
            for (address, balance) in balance_increments {
                if balance == 0 {
                    continue;
                }

                let account = db
                    .load_account(address)
                    .map_err(|err| StoreError::Custom(format!("revm CacheDB error: {err}")))?;

                account.info.balance += RevmU256::from(balance);
                if account.account_state == RevmAccountState::None {
                    account.account_state = RevmAccountState::Touched;
                }
            }
        }
    }
    Ok(())
}

pub fn block_env(header: &BlockHeader, spec_id: SpecId) -> BlockEnv {
    BlockEnv {
        number: RevmU256::from(header.number),
        coinbase: RevmAddress(header.coinbase.0.into()),
        timestamp: RevmU256::from(header.timestamp),
        gas_limit: RevmU256::from(header.gas_limit),
        basefee: RevmU256::from(header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE)),
        difficulty: RevmU256::from_limbs(header.difficulty.0),
        prevrandao: Some(header.prev_randao.as_fixed_bytes().into()),
        blob_excess_gas_and_price: Some(BlobExcessGasAndPrice::new(
            header.excess_blob_gas.unwrap_or_default(),
            spec_id >= SpecId::PRAGUE,
        )),
    }
}

// Used for the L2
pub const DEPOSIT_MAGIC_DATA: &[u8] = b"mint";
pub fn tx_env(tx: &Transaction, sender: Address) -> TxEnv {
    let max_fee_per_blob_gas = tx
        .max_fee_per_blob_gas()
        .map(|x| RevmU256::from_be_bytes(x.to_big_endian()));
    TxEnv {
        caller: match tx {
            Transaction::PrivilegedL2Transaction(_tx) => RevmAddress::ZERO,
            _ => RevmAddress(sender.0.into()),
        },
        gas_limit: tx.gas_limit(),
        gas_price: RevmU256::from(tx.gas_price()),
        transact_to: match tx.to() {
            TxKind::Call(address) => RevmTxKind::Call(address.0.into()),
            TxKind::Create => RevmTxKind::Create,
        },
        value: RevmU256::from_limbs(tx.value().0),
        data: match tx {
            Transaction::PrivilegedL2Transaction(_tx) => DEPOSIT_MAGIC_DATA.into(),
            _ => tx.data().clone().into(),
        },
        nonce: Some(tx.nonce()),
        chain_id: tx.chain_id(),
        access_list: tx
            .access_list()
            .into_iter()
            .map(|(addr, list)| {
                let (address, storage_keys) = (
                    RevmAddress(addr.0.into()),
                    list.into_iter()
                        .map(|a| FixedBytes::from_slice(a.as_bytes()))
                        .collect(),
                );
                AccessListItem {
                    address,
                    storage_keys,
                }
            })
            .collect(),
        gas_priority_fee: tx.max_priority_fee().map(RevmU256::from),
        blob_hashes: tx
            .blob_versioned_hashes()
            .into_iter()
            .map(|hash| B256::from(hash.0))
            .collect(),
        max_fee_per_blob_gas,
        // EIP7702
        // https://eips.ethereum.org/EIPS/eip-7702
        // The latest version of revm(19.3.0) is needed to run with the latest changes.
        // NOTE:
        // - rust 1.82.X is needed
        // - rust-toolchain 1.82.X is needed (this can be found in ethrex/crates/vm/levm/rust-toolchain.toml)
        authorization_list: tx.authorization_list().map(|list| {
            list.into_iter()
                .map(|auth_t| {
                    SignedAuthorization::new_unchecked(
                        RevmAuthorization {
                            chain_id: RevmU256::from_limbs(auth_t.chain_id.0),
                            address: RevmAddress(auth_t.address.0.into()),
                            nonce: auth_t.nonce,
                        },
                        auth_t.y_parity.as_u32() as u8,
                        RevmU256::from_le_bytes(auth_t.r_signature.to_little_endian()),
                        RevmU256::from_le_bytes(auth_t.s_signature.to_little_endian()),
                    )
                })
                .collect::<Vec<SignedAuthorization>>()
                .into()
        }),
    }
}

// Used to estimate gas and create access lists
pub(crate) fn tx_env_from_generic(tx: &GenericTransaction, basefee: u64) -> TxEnv {
    let gas_price = calculate_gas_price(tx, basefee);
    TxEnv {
        caller: RevmAddress(tx.from.0.into()),
        gas_limit: tx.gas.unwrap_or(u64::MAX), // Ensure tx doesn't fail due to gas limit
        gas_price,
        transact_to: match tx.to {
            TxKind::Call(address) => RevmTxKind::Call(address.0.into()),
            TxKind::Create => RevmTxKind::Create,
        },
        value: RevmU256::from_limbs(tx.value.0),
        data: tx.input.clone().into(),
        nonce: tx.nonce,
        chain_id: tx.chain_id,
        access_list: tx
            .access_list
            .iter()
            .map(|list| {
                let (address, storage_keys) = (
                    RevmAddress::from_slice(list.address.as_bytes()),
                    list.storage_keys
                        .iter()
                        .map(|a| FixedBytes::from_slice(a.as_bytes()))
                        .collect(),
                );
                AccessListItem {
                    address,
                    storage_keys,
                }
            })
            .collect(),
        gas_priority_fee: tx.max_priority_fee_per_gas.map(RevmU256::from),
        blob_hashes: tx
            .blob_versioned_hashes
            .iter()
            .map(|hash| B256::from(hash.0))
            .collect(),
        max_fee_per_blob_gas: tx.max_fee_per_blob_gas.map(|x| RevmU256::from_limbs(x.0)),
        // EIP7702
        // https://eips.ethereum.org/EIPS/eip-7702
        // The latest version of revm(19.3.0) is needed to run with the latest changes.
        // NOTE:
        // - rust 1.82.X is needed
        // - rust-toolchain 1.82.X is needed (this can be found in ethrex/crates/vm/levm/rust-toolchain.toml)
        authorization_list: tx.authorization_list.clone().map(|list| {
            list.into_iter()
                .map(|auth_t| {
                    SignedAuthorization::new_unchecked(
                        RevmAuthorization {
                            chain_id: RevmU256::from_le_bytes(auth_t.chain_id.to_little_endian()),
                            address: RevmAddress(auth_t.address.0.into()),
                            nonce: auth_t.nonce,
                        },
                        auth_t.y_parity.as_u32() as u8,
                        RevmU256::from_le_bytes(auth_t.r.to_little_endian()),
                        RevmU256::from_le_bytes(auth_t.s.to_little_endian()),
                    )
                })
                .collect::<Vec<SignedAuthorization>>()
                .into()
        }),
    }
}

// Creates an AccessListInspector that will collect the accesses used by the evm execution
pub(crate) fn access_list_inspector(tx_env: &TxEnv) -> Result<AccessListInspector, EvmError> {
    // Access list provided by the transaction
    let current_access_list = RevmAccessList(tx_env.access_list.clone());
    // Addresses accessed when using precompiles
    Ok(AccessListInspector::new(current_access_list))
}

/// Calculating gas_price according to EIP-1559 rules
/// See https://github.com/ethereum/go-ethereum/blob/7ee9a6e89f59cee21b5852f5f6ffa2bcfc05a25f/internal/ethapi/transaction_args.go#L430
fn calculate_gas_price(tx: &GenericTransaction, basefee: u64) -> Uint<256, 4> {
    if tx.gas_price != 0 {
        // Legacy gas field was specified, use it
        RevmU256::from(tx.gas_price)
    } else {
        // Backfill the legacy gas price for EVM execution, (zero if max_fee_per_gas is zero)
        RevmU256::from(min(
            tx.max_priority_fee_per_gas.unwrap_or(0) + basefee,
            tx.max_fee_per_gas.unwrap_or(0),
        ))
    }
}

/// When basefee tracking is disabled  (ie. env.disable_base_fee = true; env.disable_block_gas_limit = true;)
/// and no gas prices were specified, lower the basefee to 0 to avoid breaking EVM invariants (basefee < feecap)
/// See https://github.com/ethereum/go-ethereum/blob/00294e9d28151122e955c7db4344f06724295ec5/core/vm/evm.go#L137
fn adjust_disabled_base_fee(
    block_env: &mut BlockEnv,
    tx_gas_price: Uint<256, 4>,
    tx_blob_gas_price: Option<Uint<256, 4>>,
) {
    if tx_gas_price == RevmU256::from(0) {
        block_env.basefee = RevmU256::from(0);
    }
    if tx_blob_gas_price.is_some_and(|v| v == RevmU256::from(0)) {
        block_env.blob_excess_gas_and_price = None;
    }
}

pub(crate) fn generic_system_contract_revm(
    block_header: &BlockHeader,
    calldata: Bytes,
    state: &mut EvmState,
    contract_address: Address,
    system_address: Address,
) -> Result<ExecutionResult, EvmError> {
    let spec_id = spec_id(&state.chain_config()?, block_header.timestamp);
    let tx_env = TxEnv {
        caller: RevmAddress::from_slice(system_address.as_bytes()),
        transact_to: RevmTxKind::Call(RevmAddress::from_slice(contract_address.as_bytes())),
        gas_limit: 30_000_000,
        data: calldata,
        ..Default::default()
    };
    let mut block_env = block_env(block_header, spec_id);
    block_env.basefee = RevmU256::ZERO;
    block_env.gas_limit = RevmU256::from(30_000_000);

    match state {
        EvmState::Store(db) => {
            let mut evm = Evm::builder()
                .with_db(db)
                .with_block_env(block_env)
                .with_tx_env(tx_env)
                .with_spec_id(spec_id)
                .build();

            let transaction_result = evm.transact()?;
            let mut result_state = transaction_result.state;
            result_state.remove(SYSTEM_ADDRESS.as_ref());
            result_state.remove(&evm.block().coinbase);

            evm.context.evm.db.commit(result_state);

            Ok(transaction_result.result.into())
        }
        EvmState::Execution(db) => {
            let mut evm = Evm::builder()
                .with_db(db)
                .with_block_env(block_env)
                .with_tx_env(tx_env)
                .with_spec_id(spec_id)
                .build();

            let transaction_result = evm.transact()?;
            let mut result_state = transaction_result.state;
            result_state.remove(SYSTEM_ADDRESS.as_ref());
            result_state.remove(&evm.block().coinbase);

            evm.context.evm.db.commit(result_state);

            Ok(transaction_result.result.into())
        }
    }
}

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub fn extract_all_requests(
    receipts: &[Receipt],
    state: &mut EvmState,
    header: &BlockHeader,
) -> Result<Vec<Requests>, EvmError> {
    let config = state.chain_config()?;
    let spec_id = spec_id(&config, header.timestamp);

    if spec_id < SpecId::PRAGUE {
        return Ok(Default::default());
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            return Ok(Default::default());
        }
    }

    let deposits = Requests::from_deposit_receipts(config.deposit_contract_address, receipts);
    let withdrawals_data = REVM::read_withdrawal_requests(header, state);
    let consolidation_data = REVM::dequeue_consolidation_requests(header, state);

    let withdrawals = Requests::from_withdrawals_data(withdrawals_data.unwrap_or_default());
    let consolidation = Requests::from_consolidation_data(consolidation_data.unwrap_or_default());

    Ok(vec![deposits, withdrawals, consolidation])
}
