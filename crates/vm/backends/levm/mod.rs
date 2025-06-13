pub mod db;
mod tracing;

use super::BlockExecutionResult;
use crate::constants::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::{EvmError, ExecutionResult};
use bytes::Bytes;
use ethrex_common::{
    types::{
        requests::Requests, AccessList, AccountUpdate, AuthorizationTuple, Block, BlockHeader,
        EIP1559Transaction, EIP7702Transaction, Fork, GenericTransaction, Receipt, Transaction,
        TxKind, Withdrawal, GWEI_TO_WEI, INITIAL_BASE_FEE,
    },
    Address, H256, U256,
};
use ethrex_levm::call_frame::CallFrameBackup;
use ethrex_levm::constants::{SYS_CALL_GAS_LIMIT, TX_BASE_COST};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::errors::{InternalError, TxValidationError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::EVMConfig;
use ethrex_levm::{
    errors::{ExecutionReport, TxResult, VMError},
    vm::{Substate, VM},
    Environment,
};
use std::cmp::min;
use std::collections::HashMap;

// Export needed types
pub use ethrex_levm::db::CacheDB;
/// The struct implements the following functions:
/// [LEVM::execute_block]
/// [LEVM::execute_tx]
/// [LEVM::get_state_transitions]
/// [LEVM::process_withdrawals]
#[derive(Debug)]
pub struct LEVM;

impl LEVM {
    pub fn execute_block(
        block: &Block,
        db: &mut GeneralizedDatabase,
    ) -> Result<BlockExecutionResult, EvmError> {
        Self::prepare_block(block, db)?;

        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for (tx, tx_sender) in block.body.get_transactions_with_sender() {
            let report = Self::execute_tx(tx, tx_sender, &block.header, db)?;

            cumulative_gas_used += report.gas_used;
            let receipt = Receipt::new(
                tx.tx_type(),
                matches!(report.result.clone(), TxResult::Success),
                cumulative_gas_used,
                report.logs.clone(),
            );

            receipts.push(receipt);
        }

        if let Some(withdrawals) = &block.body.withdrawals {
            Self::process_withdrawals(db, withdrawals)?;
        }

        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                let requests = extract_all_requests_levm(&receipts, db, &block.header)?;
            } else {
                let requests = Default::default();
            }
        }

        Ok(BlockExecutionResult { receipts, requests })
    }

    fn setup_env(
        tx: &Transaction,
        tx_sender: Address,
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<Environment, EvmError> {
        let chain_config = db.store.get_chain_config()?;
        let gas_price: U256 = tx
            .effective_gas_price(block_header.base_fee_per_gas)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientMaxFeePerGas,
            ))?
            .into();

        let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
        let env = Environment {
            origin: tx_sender,
            gas_limit: tx.gas_limit(),
            config,
            block_number: block_header.number.into(),
            coinbase: block_header.coinbase,
            timestamp: block_header.timestamp.into(),
            prev_randao: Some(block_header.prev_randao),
            chain_id: chain_config.chain_id.into(),
            base_fee_per_gas: block_header.base_fee_per_gas.unwrap_or_default().into(),
            gas_price,
            block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
            block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
            tx_blob_hashes: tx.blob_versioned_hashes(),
            tx_max_priority_fee_per_gas: tx.max_priority_fee().map(U256::from),
            tx_max_fee_per_gas: tx.max_fee_per_gas().map(U256::from),
            tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas(),
            tx_nonce: tx.nonce(),
            block_gas_limit: block_header.gas_limit,
            difficulty: block_header.difficulty,
            is_privileged: matches!(tx, Transaction::PrivilegedL2Transaction(_)),
        };

        Ok(env)
    }

    pub fn execute_tx(
        // The transaction to execute.
        tx: &Transaction,
        // The transactions recovered address
        tx_sender: Address,
        // The block header for the current block.
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<ExecutionReport, EvmError> {
        let env = Self::setup_env(tx, tx_sender, block_header, db)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled());

        vm.execute().map_err(VMError::into)
    }

    pub fn execute_tx_l2(
        // The transaction to execute.
        tx: &Transaction,
        // The transactions recovered address
        tx_sender: Address,
        // The block header for the current block.
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<(ExecutionReport, CallFrameBackup), EvmError> {
        let env = Self::setup_env(tx, tx_sender, block_header, db)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled());

        let report_result = vm.execute().map_err(EvmError::from)?;

        // Here we differ from the execute_tx function from the L1.
        // We need to check if the transaction exceeded the blob size limit.
        // If it did, we need to revert the state changes made by the transaction and return the error.
        let call_frame_backup = vm
            .call_frames
            .pop()
            .ok_or(VMError::Internal(InternalError::CallFrame))?
            .call_frame_backup;

        Ok((report_result, call_frame_backup))
    }

    pub fn undo_last_tx(db: &mut GeneralizedDatabase) -> Result<(), EvmError> {
        db.undo_last_transaction()?;
        Ok(())
    }

    pub fn simulate_tx_from_generic(
        // The transaction to execute.
        tx: &GenericTransaction,
        // The block header for the current block.
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<ExecutionResult, EvmError> {
        let mut env = env_from_generic(tx, block_header, db)?;

        env.block_gas_limit = u64::MAX; // disable block gas limit

        adjust_disabled_base_fee(&mut env);

        let mut vm = vm_from_generic(tx, env, db)?;

        vm.execute()
            .map(|value| value.into())
            .map_err(VMError::into)
    }

    pub fn get_state_transitions(
        db: &mut GeneralizedDatabase,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        let mut account_updates: Vec<AccountUpdate> = vec![];
        for (address, new_state_account) in db.cache.iter() {
            // In case the account is not in immutable_cache (rare) we search for it in the actual database.
            let initial_state_account =
                db.immutable_cache
                    .get(address)
                    .ok_or(EvmError::Custom(format!(
                        "Failed to get account {address} from immutable cache",
                    )))?;

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
                Some(new_state_account.code.clone())
            } else {
                None
            };

            // 2. Storage has been updated if the current value is different from the one before execution.
            let mut added_storage = HashMap::new();

            for (key, new_value) in &new_state_account.storage {
                let old_value = initial_state_account.storage.get(key).ok_or_else(|| { EvmError::Custom(format!("Failed to get old value from account's initial storage for address: {address}"))})?;

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
            let removed = new_state_account.is_empty();

            if !removed && !acc_info_updated && !storage_updated {
                // Account hasn't been updated
                continue;
            }

            let account_update = AccountUpdate {
                address: *address,
                removed,
                info,
                code,
                added_storage,
            };

            account_updates.push(account_update);
        }
        db.cache.clear();
        db.immutable_cache.clear();
        Ok(account_updates)
    }

    pub fn process_withdrawals(
        db: &mut GeneralizedDatabase,
        withdrawals: &[Withdrawal],
    ) -> Result<(), EvmError> {
        // For every withdrawal we increment the target account's balance
        for (address, increment) in withdrawals
            .iter()
            .filter(|withdrawal| withdrawal.amount > 0)
            .map(|w| (w.address, u128::from(w.amount) * u128::from(GWEI_TO_WEI)))
        {
            let mut account = db
                .get_account(address)
                .map_err(|_| EvmError::DB(format!("Withdrawal account {address} not found")))?
                .clone(); // Not a big deal cloning here because it's an EOA.

            account.info.balance += increment.into();
            db.cache.insert(address, account);
        }
        Ok(())
    }

    // SYSTEM CONTRACTS
    pub fn beacon_root_contract_call(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<(), EvmError> {
        let beacon_root = match block_header.parent_beacon_block_root {
            None => {
                return Err(EvmError::Header(
                    "parent_beacon_block_root field is missing".to_string(),
                ))
            }
            Some(beacon_root) => beacon_root,
        };

        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(beacon_root.as_bytes()),
            db,
            *BEACON_ROOTS_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }

    pub fn process_block_hash_history(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<(), EvmError> {
        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(block_header.parent_hash.as_bytes()),
            db,
            *HISTORY_STORAGE_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    pub(crate) fn read_withdrawal_requests(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<ExecutionReport, EvmError> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db,
            *WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;

        // According to EIP-7002 we need to check if the WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS
        // has any code after being deployed. If not, the whole block becomes invalid.
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7002.md
        let account = db.get_account(*WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS)?;
        if !account.has_code() {
            return Err(EvmError::SystemContractEmpty(
                "WITHDRAWAL_REQUEST_PREDEPLOY".to_string(),
            ));
        }

        match report.result {
            TxResult::Success => Ok(report),
            // EIP-7002 specifies that a failed system call invalidates the entire block.
            TxResult::Revert(vm_error) => Err(EvmError::SystemContractCallFailed(format!(
                "REVERT when reading withdrawal requests with error: {:?}. According to EIP-7002, the revert of this system call invalidates the block.",
                vm_error
            ))),
        }
    }
    pub(crate) fn dequeue_consolidation_requests(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<ExecutionReport, EvmError> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db,
            *CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;

        // According to EIP-7251 we need to check if the CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS
        // has any code after being deployed. If not, the whole block becomes invalid.
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7251.md
        let acc = db.get_account(*CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS)?;
        if !acc.has_code() {
            return Err(EvmError::SystemContractEmpty(
                "CONSOLIDATION_REQUEST_PREDEPLOY".to_string(),
            ));
        }

        match report.result {
            TxResult::Success => Ok(report),
            // EIP-7251 specifies that a failed system call invalidates the entire block.
            TxResult::Revert(vm_error) => Err(EvmError::SystemContractCallFailed(format!(
                "REVERT when dequeuing consolidation requests with error: {:?}. According to EIP-7251, the revert of this system call invalidates the block.",
                vm_error
            ))),
        }
    }

    pub fn create_access_list(
        mut tx: GenericTransaction,
        header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<(ExecutionResult, AccessList), VMError> {
        let mut env = env_from_generic(&tx, header, db)?;

        adjust_disabled_base_fee(&mut env);

        let mut vm = vm_from_generic(&tx, env.clone(), db)?;

        vm.stateless_execute()?;
        let access_list = build_access_list(&vm.substate);

        // Execute the tx again, now with the created access list.
        tx.access_list = access_list.iter().map(|item| item.into()).collect();
        let mut vm = vm_from_generic(&tx, env.clone(), db)?;

        let report = vm.stateless_execute()?;

        Ok((report.into(), access_list))
    }

    pub fn prepare_block(block: &Block, db: &mut GeneralizedDatabase) -> Result<(), EvmError> {
        let chain_config = db.store.get_chain_config()?;
        let block_header = &block.header;
        let fork = chain_config.fork(block_header.timestamp);

        if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
            #[cfg(not(feature = "l2"))]
            Self::beacon_root_contract_call(block_header, db)?;
        }

        if fork >= Fork::Prague {
            //eip 2935: stores parent block hash in system contract
            #[cfg(not(feature = "l2"))]
            Self::process_block_hash_history(block_header, db)?;
        }
        Ok(())
    }
}

pub fn generic_system_contract_levm(
    block_header: &BlockHeader,
    calldata: Bytes,
    db: &mut GeneralizedDatabase,
    contract_address: Address,
    system_address: Address,
) -> Result<ExecutionReport, EvmError> {
    let chain_config = db.store.get_chain_config()?;
    let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
    let system_account_backup = db.cache.get(&system_address).cloned();
    let coinbase_backup = db.cache.get(&block_header.coinbase).cloned();
    let env = Environment {
        origin: system_address,
        // EIPs 2935, 4788, 7002 and 7251 dictate that the system calls have a gas limit of 30 million and they do not use intrinsic gas.
        // So we add the base cost that will be taken in the execution.
        gas_limit: SYS_CALL_GAS_LIMIT + TX_BASE_COST,
        block_number: block_header.number.into(),
        coinbase: block_header.coinbase,
        timestamp: block_header.timestamp.into(),
        prev_randao: Some(block_header.prev_randao),
        base_fee_per_gas: U256::zero(),
        gas_price: U256::zero(),
        block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
        block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
        block_gas_limit: u64::MAX, // System calls, have no constraint on the block's gas limit.
        config,
        ..Default::default()
    };

    let tx = &Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract_address),
        value: U256::zero(),
        data: calldata,
        ..Default::default()
    });
    let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled());

    let report = vm.execute().map_err(EvmError::from)?;

    if let Some(system_account) = system_account_backup {
        db.cache.insert(system_address, system_account);
    } else {
        // If the system account was not in the cache, we need to remove it
        db.cache.remove(&system_address);
    }

    if let Some(coinbase_account) = coinbase_backup {
        db.cache.insert(block_header.coinbase, coinbase_account);
    } else {
        // If the coinbase account was not in the cache, we need to remove it
        db.cache.remove(&block_header.coinbase);
    }

    Ok(report)
}

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub fn extract_all_requests_levm(
    receipts: &[Receipt],
    db: &mut GeneralizedDatabase,
    header: &BlockHeader,
) -> Result<Vec<Requests>, EvmError> {
    let chain_config = db.store.get_chain_config()?;
    let fork = chain_config.fork(header.timestamp);

    if fork < Fork::Prague {
        return Ok(Default::default());
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            return Ok(Default::default());
        }
    }

    let withdrawals_data: Vec<u8> = LEVM::read_withdrawal_requests(header, db)?.output.into();
    let consolidation_data: Vec<u8> = LEVM::dequeue_consolidation_requests(header, db)?
        .output
        .into();

    let deposits = Requests::from_deposit_receipts(chain_config.deposit_contract_address, receipts)
        .ok_or(EvmError::InvalidDepositRequest)?;
    let withdrawals = Requests::from_withdrawals_data(withdrawals_data);
    let consolidation = Requests::from_consolidation_data(consolidation_data);

    Ok(vec![deposits, withdrawals, consolidation])
}

/// Calculating gas_price according to EIP-1559 rules
/// See https://github.com/ethereum/go-ethereum/blob/7ee9a6e89f59cee21b5852f5f6ffa2bcfc05a25f/internal/ethapi/transaction_args.go#L430
pub fn calculate_gas_price(tx: &GenericTransaction, basefee: u64) -> U256 {
    if tx.gas_price != 0 {
        // Legacy gas field was specified, use it
        tx.gas_price.into()
    } else {
        // Backfill the legacy gas price for EVM execution, (zero if max_fee_per_gas is zero)
        min(
            tx.max_priority_fee_per_gas.unwrap_or(0) + basefee,
            tx.max_fee_per_gas.unwrap_or(0),
        )
        .into()
    }
}

/// When basefee tracking is disabled  (ie. env.disable_base_fee = true; env.disable_block_gas_limit = true;)
/// and no gas prices were specified, lower the basefee to 0 to avoid breaking EVM invariants (basefee < feecap)
/// See https://github.com/ethereum/go-ethereum/blob/00294e9d28151122e955c7db4344f06724295ec5/core/vm/evm.go#L137
fn adjust_disabled_base_fee(env: &mut Environment) {
    if env.gas_price == U256::zero() {
        env.base_fee_per_gas = U256::zero();
    }
    if env
        .tx_max_fee_per_blob_gas
        .is_some_and(|v| v == U256::zero())
    {
        env.block_excess_blob_gas = None;
    }
}

pub fn build_access_list(substate: &Substate) -> AccessList {
    let access_list: AccessList = substate
        .accessed_storage_slots
        .iter()
        .map(|(address, slots)| (*address, slots.iter().cloned().collect::<Vec<H256>>()))
        .collect();

    access_list
}

fn env_from_generic(
    tx: &GenericTransaction,
    header: &BlockHeader,
    db: &GeneralizedDatabase,
) -> Result<Environment, VMError> {
    let chain_config = db.store.get_chain_config()?;
    let gas_price = calculate_gas_price(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    let config = EVMConfig::new_from_chain_config(&chain_config, header);
    Ok(Environment {
        origin: tx.from.0.into(),
        gas_limit: tx.gas.unwrap_or(header.gas_limit), // Ensure tx doesn't fail due to gas limit
        config,
        block_number: header.number.into(),
        coinbase: header.coinbase,
        timestamp: header.timestamp.into(),
        prev_randao: Some(header.prev_randao),
        chain_id: chain_config.chain_id.into(),
        base_fee_per_gas: header.base_fee_per_gas.unwrap_or_default().into(),
        gas_price,
        block_excess_blob_gas: header.excess_blob_gas.map(U256::from),
        block_blob_gas_used: header.blob_gas_used.map(U256::from),
        tx_blob_hashes: tx.blob_versioned_hashes.clone(),
        tx_max_priority_fee_per_gas: tx.max_priority_fee_per_gas.map(U256::from),
        tx_max_fee_per_gas: tx.max_fee_per_gas.map(U256::from),
        tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas,
        tx_nonce: tx.nonce.unwrap_or_default(),
        block_gas_limit: header.gas_limit,
        difficulty: header.difficulty,
        is_privileged: false,
    })
}

fn vm_from_generic<'a>(
    tx: &GenericTransaction,
    env: Environment,
    db: &'a mut GeneralizedDatabase,
) -> Result<VM<'a>, VMError> {
    let tx = match &tx.authorization_list {
        Some(authorization_list) => Transaction::EIP7702Transaction(EIP7702Transaction {
            to: match tx.to {
                TxKind::Call(to) => to,
                TxKind::Create => {
                    return Err(InternalError::msg("Generic Tx cannot be create type").into())
                }
            },
            value: tx.value,
            data: tx.input.clone(),
            access_list: tx
                .access_list
                .iter()
                .map(|list| (list.address, list.storage_keys.clone()))
                .collect(),
            authorization_list: authorization_list
                .iter()
                .map(|auth| Into::<AuthorizationTuple>::into(auth.clone()))
                .collect(),
            ..Default::default()
        }),
        None => Transaction::EIP1559Transaction(EIP1559Transaction {
            to: tx.to.clone(),
            value: tx.value,
            data: tx.input.clone(),
            access_list: tx
                .access_list
                .iter()
                .map(|list| (list.address, list.storage_keys.clone()))
                .collect(),
            ..Default::default()
        }),
    };
    Ok(VM::new(env, db, &tx, LevmCallTracer::disabled()))
}
