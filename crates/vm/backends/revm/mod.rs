pub mod db;
pub mod helpers;
mod tracing;

use super::BlockExecutionResult;
use crate::backends::revm::db::EvmState;
use crate::constants::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::errors::EvmError;
use crate::execution_result::ExecutionResult;
use crate::helpers::spec_id;
use ethrex_common::types::{AccountInfo, AccountUpdate};
use ethrex_common::{BigEndianHash, H256, U256};
use ethrex_levm::constants::{SYS_CALL_GAS_LIMIT, TX_BASE_COST};

use revm::db::AccountStatus;
use revm::db::states::bundle_state::BundleRetention;

use revm::{Database, DatabaseCommit};
use revm::{
    Evm,
    primitives::{B256, BlobExcessGasAndPrice, BlockEnv, TxEnv},
};
use revm_inspectors::access_list::AccessListInspector;
// Rename imported types for clarity
use ethrex_common::{
    Address,
    types::{
        Block, BlockHeader, GWEI_TO_WEI, GenericTransaction, INITIAL_BASE_FEE, Receipt,
        Transaction, TxKind, Withdrawal, requests::Requests,
    },
};
use revm_primitives::Bytes;
use revm_primitives::{
    AccessList as RevmAccessList, AccessListItem, Address as RevmAddress,
    Authorization as RevmAuthorization, FixedBytes, SignedAuthorization, SpecId,
    TxKind as RevmTxKind, U256 as RevmU256, ruint::Uint,
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
        let spec_id: SpecId = spec_id(
            &state.inner.database.get_chain_config()?,
            block_header.timestamp,
        );

        if block_header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
            Self::beacon_root_contract_call(block_header, state)?;
        }

        //eip 2935: stores parent block hash in system contract
        if spec_id >= SpecId::PRAGUE {
            Self::process_block_hash_history(block_header, state)?;
        }

        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for (tx, sender) in block.body.get_transactions_with_sender().map_err(|error| {
            EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
        })? {
            let result = Self::execute_tx(tx, block_header, state, spec_id, sender)?;
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

        let requests = extract_all_requests(&receipts, state, block_header)?;

        Ok(BlockExecutionResult { receipts, requests })
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
    ) -> Result<(), EvmError> {
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
        initial_state.inner.increment_balances(balance_increments)?;
        Ok(())
    }

    // SYSTEM CONTRACTS
    pub fn beacon_root_contract_call(
        block_header: &BlockHeader,
        state: &mut EvmState,
    ) -> Result<(), EvmError> {
        let beacon_root = block_header.parent_beacon_block_root.ok_or_else(|| {
            EvmError::Header("parent_beacon_block_root field is missing".to_string())
        })?;

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
    fn system_contract_account_info(
        addr: Address,
        state: &mut EvmState,
    ) -> Result<revm_primitives::AccountInfo, EvmError> {
        let revm_addr = RevmAddress::from_slice(addr.as_bytes());
        let account_info = state.inner.basic(revm_addr)?.ok_or(EvmError::DB(
            "System contract address was not found after deployment".to_string(),
        ))?;
        Ok(account_info)
    }
    pub(crate) fn read_withdrawal_requests(
        block_header: &BlockHeader,
        state: &mut EvmState,
    ) -> Result<Vec<u8>, EvmError> {
        let tx_result = generic_system_contract_revm(
            block_header,
            Bytes::new(),
            state,
            *WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;

        // According to EIP-7002 we need to check if the WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS
        // has any code after being deployed. If not, the whole block becomes invalid.
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7002.md
        let account_info =
            Self::system_contract_account_info(*WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS, state)?;
        if account_info.is_empty_code_hash() {
            return Err(EvmError::SystemContractEmpty(
                "WITHDRAWAL_REQUEST_PREDEPLOY".to_string(),
            ));
        }

        match tx_result {
            ExecutionResult::Success {
                gas_used: _,
                gas_refunded: _,
                logs: _,
                output,
            } => Ok(output.into()),
            // EIP-7002 specifies that a failed system call invalidates the entire block.
            ExecutionResult::Halt { reason, gas_used } => {
                let err_str = format!(
                    "Transaction HALT when calling WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS with reason: {reason} and with used gas: {gas_used}"
                );
                Err(EvmError::SystemContractCallFailed(err_str))
            }
            ExecutionResult::Revert { gas_used, output } => {
                let err_str = format!(
                    "Transaction REVERT when calling WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS with output: {output:?} and with used gas: {gas_used}",
                );
                Err(EvmError::SystemContractCallFailed(err_str))
            }
        }
    }
    pub(crate) fn dequeue_consolidation_requests(
        block_header: &BlockHeader,
        state: &mut EvmState,
    ) -> Result<Vec<u8>, EvmError> {
        let tx_result = generic_system_contract_revm(
            block_header,
            Bytes::new(),
            state,
            *CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;

        // According to EIP-7251 we need to check if the CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS
        // has any code after being deployed. If not, the whole block becomes invalid.
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7251.md
        let account_info =
            Self::system_contract_account_info(*CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, state)?;
        if account_info.is_empty_code_hash() {
            return Err(EvmError::SystemContractEmpty(
                "CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS".to_string(),
            ));
        }

        match tx_result {
            ExecutionResult::Success {
                gas_used: _,
                gas_refunded: _,
                logs: _,
                output,
            } => Ok(output.into()),
            // EIP-7251 specifies that a failed system call invalidates the entire block.
            ExecutionResult::Halt { reason, gas_used } => {
                let err_str = format!(
                    "Transaction HALT when calling CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS with reason: {reason} and with used gas: {gas_used}"
                );
                Err(EvmError::SystemContractCallFailed(err_str))
            }
            ExecutionResult::Revert { gas_used, output } => {
                let err_str = format!(
                    "Transaction REVERT when calling CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS with output: {output:?} and with used gas: {gas_used}",
                );
                Err(EvmError::SystemContractCallFailed(err_str))
            }
        }
    }

    /// Gets the state_transitions == [AccountUpdate] from the [EvmState].
    pub fn get_state_transitions(
        initial_state: &mut EvmState,
    ) -> Vec<ethrex_common::types::AccountUpdate> {
        let initial_state = &mut initial_state.inner;
        initial_state.merge_transitions(BundleRetention::PlainState);
        let bundle = initial_state.take_bundle();

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

            // Edge case: Account was destroyed and created again afterwards with CREATE2.
            if matches!(account.status, AccountStatus::DestroyedChanged) {
                // Push to account updates the removal of the account and then push the new state of the account.
                // This is for clearing the account's storage when it was selfdestructed in the first place.
                account_updates.push(AccountUpdate::removed(address));
                // This will always be Some though, because it is DestroyedChanged
                if let Some(new_acc_info) = account.account_info() {
                    let new_acc_update = AccountUpdate {
                        address,
                        removed: false,
                        info: Some(AccountInfo {
                            code_hash: H256::from_slice(new_acc_info.code_hash.as_slice()),
                            balance: U256::from_little_endian(new_acc_info.balance.as_le_slice()),
                            nonce: new_acc_info.nonce,
                        }),
                        code: new_acc_info.code.map(|c| c.original_bytes().0),
                        added_storage: account
                            .storage
                            .iter()
                            .map(|(key, slot)| {
                                (
                                    H256::from_uint(&U256::from_little_endian(key.as_le_slice())),
                                    U256::from_little_endian(slot.present_value().as_le_slice()),
                                )
                            })
                            .collect(),
                    };
                    account_updates.push(new_acc_update);
                }
                continue;
            }
            // Apply account changes to DB
            let mut account_update = AccountUpdate::new(address);
            // If the account was changed then both original and current info will be present in the bundle account
            if account.is_info_changed()
                && let Some(new_acc_info) = account.account_info()
            {
                // Update account info in DB
                let code_hash = H256::from_slice(new_acc_info.code_hash.as_slice());
                let account_info = AccountInfo {
                    code_hash,
                    balance: U256::from_little_endian(new_acc_info.balance.as_le_slice()),
                    nonce: new_acc_info.nonce,
                };
                account_update.info = Some(account_info);
                // Update code in db
                if account.is_contract_changed()
                    && let Some(code) = new_acc_info.code
                {
                    account_update.code = Some(code.original_bytes().0);
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
    let chain_config = state.inner.database.get_chain_config()?;
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
    let tx_result = {
        let mut evm = evm_builder.with_db(&mut state.inner).build();
        evm.transact().map_err(EvmError::from)?
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
    let state = &mut state.inner;
    let tx_result = {
        let chain_spec = state.database.get_chain_config()?;
        #[allow(unused_mut)]
        let mut evm_builder = Evm::builder()
            .with_block_env(block_env)
            .with_tx_env(tx_env)
            .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain_id)
            .with_spec_id(spec_id);

        let mut evm = evm_builder.with_db(state).build();
        evm.transact_commit().map_err(EvmError::from)?
    };
    Ok(tx_result.into())
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
        caller: RevmAddress(sender.0.into()),
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
            .iter()
            .map(|(addr, list)| {
                let (address, storage_keys) = (
                    RevmAddress(addr.0.into()),
                    list.iter()
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
            list.iter()
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
    let state = &mut state.inner;
    let spec_id = spec_id(&state.database.get_chain_config()?, block_header.timestamp);
    let tx_env = TxEnv {
        caller: RevmAddress::from_slice(system_address.as_bytes()),
        transact_to: RevmTxKind::Call(RevmAddress::from_slice(contract_address.as_bytes())),
        // EIPs 2935, 4788, 7002 and 7251 dictate that the system calls have a gas limit of 30 million and they do not use intrinsic gas.
        // So we add the base cost that will be taken in the execution.
        gas_limit: SYS_CALL_GAS_LIMIT + TX_BASE_COST,
        data: calldata,
        ..Default::default()
    };
    let mut block_env = block_env(block_header, spec_id);
    block_env.basefee = RevmU256::ZERO;
    block_env.gas_limit = RevmU256::from(u64::MAX); // System calls, have no constraint on the block's gas limit.

    let mut evm = Evm::builder()
        .with_db(state)
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

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub fn extract_all_requests(
    receipts: &[Receipt],
    state: &mut EvmState,
    header: &BlockHeader,
) -> Result<Vec<Requests>, EvmError> {
    let config = state.inner.database.get_chain_config()?;
    let spec_id = spec_id(&config, header.timestamp);

    if spec_id < SpecId::PRAGUE {
        return Ok(Default::default());
    }

    let deposits = Requests::from_deposit_receipts(config.deposit_contract_address, receipts)
        .ok_or(EvmError::InvalidDepositRequest)?;
    let withdrawals_data = REVM::read_withdrawal_requests(header, state)?;
    let consolidation_data = REVM::dequeue_consolidation_requests(header, state)?;

    let withdrawals = Requests::from_withdrawals_data(withdrawals_data);
    let consolidation = Requests::from_consolidation_data(consolidation_data);

    Ok(vec![deposits, withdrawals, consolidation])
}
