use crate::{
    report::{EFTestReport, TestVector},
    runner::{EFTestRunnerError, InternalError},
    types::{EFTest, TransactionExpectedException},
    utils::{self, effective_gas_price},
};
use ethrex_core::{
    types::{code_hash, AccountInfo},
    H256, U256,
};
use ethrex_levm::{
    db::CacheDB,
    errors::{TransactionReport, TxValidationError, VMError},
    vm::VM,
    Environment,
};
use ethrex_storage::AccountUpdate;
use ethrex_vm::{db::StoreWrapper, EvmState};
use keccak_hash::keccak;
use std::{collections::HashMap, sync::Arc};

pub fn run_ef_test(test: &EFTest) -> Result<EFTestReport, EFTestRunnerError> {
    let mut ef_test_report = EFTestReport::new(
        test.name.clone(),
        test.dir.clone(),
        test._info.generated_test_hash,
        test.fork(),
    );
    for (vector, _tx) in test.transactions.iter() {
        match run_ef_test_tx(vector, test) {
            Ok(_) => continue,
            Err(EFTestRunnerError::VMInitializationFailed(reason)) => {
                ef_test_report.register_vm_initialization_failure(reason, *vector);
            }
            Err(EFTestRunnerError::FailedToEnsurePreState(reason)) => {
                ef_test_report.register_pre_state_validation_failure(reason, *vector);
            }
            Err(EFTestRunnerError::ExecutionFailedUnexpectedly(error)) => {
                ef_test_report.register_unexpected_execution_failure(error, *vector);
            }
            Err(EFTestRunnerError::FailedToEnsurePostState(transaction_report, reason)) => {
                ef_test_report.register_post_state_validation_failure(
                    transaction_report,
                    reason,
                    *vector,
                );
            }
            Err(EFTestRunnerError::VMExecutionMismatch(_)) => {
                return Err(EFTestRunnerError::Internal(InternalError::FirstRunInternal(
                    "VM execution mismatch errors should only happen when running with revm. This failed during levm's execution."
                        .to_owned(),
                )));
            }
            Err(EFTestRunnerError::ExpectedExceptionDoesNotMatchReceived(reason)) => {
                ef_test_report.register_post_state_validation_error_mismatch(reason, *vector);
            }
            Err(EFTestRunnerError::Internal(reason)) => {
                return Err(EFTestRunnerError::Internal(reason));
            }
        }
    }
    Ok(ef_test_report)
}

pub fn run_ef_test_tx(vector: &TestVector, test: &EFTest) -> Result<(), EFTestRunnerError> {
    let mut levm = prepare_vm_for_tx(vector, test)?;
    ensure_pre_state(&levm, test)?;
    let levm_execution_result = levm.transact();
    ensure_post_state(&levm_execution_result, vector, test)?;
    Ok(())
}

pub fn prepare_vm_for_tx(vector: &TestVector, test: &EFTest) -> Result<VM, EFTestRunnerError> {
    let (initial_state, block_hash) = utils::load_initial_state(test);
    let db = Arc::new(StoreWrapper {
        store: initial_state.database().unwrap().clone(),
        block_hash,
    });

    let tx = test
        .transactions
        .get(vector)
        .ok_or(EFTestRunnerError::Internal(
            InternalError::FirstRunInternal("Failed to get transaction".to_owned()),
        ))?;

    let access_lists = tx
        .access_list
        .iter()
        .map(|arg| (arg.address, arg.storage_keys.clone()))
        .collect();

    VM::new(
        tx.to.clone(),
        Environment {
            origin: tx.sender,
            refunded_gas: 0,
            gas_limit: tx.gas_limit,
            block_number: test.env.current_number,
            coinbase: test.env.current_coinbase,
            timestamp: test.env.current_timestamp,
            prev_randao: test.env.current_random,
            chain_id: U256::from(1729),
            base_fee_per_gas: test.env.current_base_fee.unwrap_or_default(),
            gas_price: effective_gas_price(test, &tx)?,
            block_excess_blob_gas: test.env.current_excess_blob_gas,
            block_blob_gas_used: None,
            tx_blob_hashes: tx.blob_versioned_hashes.clone(),
            tx_max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
            tx_max_fee_per_gas: tx.max_fee_per_gas,
            tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas,
            block_gas_limit: test.env.current_gas_limit.as_u64(),
        },
        tx.value,
        tx.data.clone(),
        db,
        CacheDB::default(),
        access_lists,
    )
    .map_err(|err| EFTestRunnerError::VMInitializationFailed(err.to_string()))
}

pub fn ensure_pre_state(evm: &VM, test: &EFTest) -> Result<(), EFTestRunnerError> {
    let world_state = &evm.db;
    for (address, pre_value) in &test.pre.0 {
        let account = world_state.get_account_info(*address);
        ensure_pre_state_condition(
            account.nonce == pre_value.nonce.as_u64(),
            format!(
                "Nonce mismatch for account {:#x}: expected {}, got {}",
                address, pre_value.nonce, account.nonce
            ),
        )?;
        ensure_pre_state_condition(
            account.balance == pre_value.balance,
            format!(
                "Balance mismatch for account {:#x}: expected {}, got {}",
                address, pre_value.balance, account.balance
            ),
        )?;
        for (k, v) in &pre_value.storage {
            let mut key_bytes = [0u8; 32];
            k.to_big_endian(&mut key_bytes);
            let storage_slot = world_state.get_storage_slot(*address, H256::from_slice(&key_bytes));
            ensure_pre_state_condition(
                &storage_slot == v,
                format!(
                    "Storage slot mismatch for account {:#x} at key {:?}: expected {}, got {}",
                    address, k, v, storage_slot
                ),
            )?;
        }
        ensure_pre_state_condition(
            keccak(account.bytecode.clone()) == keccak(pre_value.code.as_ref()),
            format!(
                "Code hash mismatch for account {:#x}: expected {}, got {}",
                address,
                keccak(pre_value.code.as_ref()),
                keccak(account.bytecode)
            ),
        )?;
    }
    Ok(())
}

fn ensure_pre_state_condition(
    condition: bool,
    error_reason: String,
) -> Result<(), EFTestRunnerError> {
    if !condition {
        return Err(EFTestRunnerError::FailedToEnsurePreState(error_reason));
    }
    Ok(())
}

// Exceptions not covered: RlpInvalidValue and Type3TxPreFork
fn exception_is_expected(
    expected_exceptions: Vec<TransactionExpectedException>,
    returned_error: VMError,
) -> bool {
    expected_exceptions.iter().any(|exception| {
        matches!(
            (exception, &returned_error),
            (
                TransactionExpectedException::IntrinsicGasTooLow,
                VMError::TxValidation(TxValidationError::IntrinsicGasTooLow)
            ) | (
                TransactionExpectedException::InsufficientAccountFunds,
                VMError::TxValidation(TxValidationError::InsufficientAccountFunds)
            ) | (
                TransactionExpectedException::PriorityGreaterThanMaxFeePerGas,
                VMError::TxValidation(TxValidationError::PriorityGreaterThanMaxFeePerGas)
            ) | (
                TransactionExpectedException::GasLimitPriceProductOverflow,
                VMError::TxValidation(TxValidationError::GasLimitPriceProductOverflow)
            ) | (
                TransactionExpectedException::SenderNotEoa,
                VMError::TxValidation(TxValidationError::SenderNotEOA)
            ) | (
                TransactionExpectedException::InsufficientMaxFeePerGas,
                VMError::TxValidation(TxValidationError::InsufficientMaxFeePerGas)
            ) | (
                TransactionExpectedException::NonceIsMax,
                VMError::TxValidation(TxValidationError::NonceIsMax)
            ) | (
                TransactionExpectedException::GasAllowanceExceeded,
                VMError::TxValidation(TxValidationError::GasAllowanceExceeded)
            ) | (
                TransactionExpectedException::Type3TxBlobCountExceeded,
                VMError::TxValidation(TxValidationError::Type3TxBlobCountExceeded)
            ) | (
                TransactionExpectedException::Type3TxZeroBlobs,
                VMError::TxValidation(TxValidationError::Type3TxZeroBlobs)
            ) | (
                TransactionExpectedException::Type3TxContractCreation,
                VMError::TxValidation(TxValidationError::Type3TxContractCreation)
            ) | (
                TransactionExpectedException::Type3TxInvalidBlobVersionedHash,
                VMError::TxValidation(TxValidationError::Type3TxInvalidBlobVersionedHash)
            ) | (
                TransactionExpectedException::InsufficientMaxFeePerBlobGas,
                VMError::TxValidation(TxValidationError::InsufficientMaxFeePerBlobGas)
            ) | (
                TransactionExpectedException::InitcodeSizeExceeded,
                VMError::TxValidation(TxValidationError::InitcodeSizeExceeded)
            )
        )
    })
}

pub fn ensure_post_state(
    levm_execution_result: &Result<TransactionReport, VMError>,
    vector: &TestVector,
    test: &EFTest,
) -> Result<(), EFTestRunnerError> {
    match levm_execution_result {
        Ok(execution_report) => {
            match test.post.vector_post_value(vector).expect_exception {
                // Execution result was successful but an exception was expected.
                Some(expected_exceptions) => {
                    // Note: expected_exceptions is a vector because can only have 1 or 2 expected errors.
                    // Here I use a match bc if there is no second position I just print the first one.
                    let error_reason = match expected_exceptions.get(1) {
                        Some(second_exception) => {
                            format!(
                                "Expected exception: {:?} or {:?}",
                                expected_exceptions.first().unwrap(),
                                second_exception
                            )
                        }
                        None => {
                            format!(
                                "Expected exception: {:?}",
                                expected_exceptions.first().unwrap()
                            )
                        }
                    };
                    return Err(EFTestRunnerError::FailedToEnsurePostState(
                        execution_report.clone(),
                        error_reason,
                    ));
                }
                // Execution result was successful and no exception was expected.
                None => {
                    let (initial_state, block_hash) = utils::load_initial_state(test);
                    let levm_account_updates =
                        get_state_transitions(&initial_state, block_hash, execution_report);
                    let pos_state_root = post_state_root(&levm_account_updates, test);
                    let expected_post_state_root_hash = test.post.vector_post_value(vector).hash;
                    if expected_post_state_root_hash != pos_state_root {
                        let error_reason = format!(
                            "Post-state root mismatch: expected {expected_post_state_root_hash:#x}, got {pos_state_root:#x}",
                        );
                        return Err(EFTestRunnerError::FailedToEnsurePostState(
                            execution_report.clone(),
                            error_reason,
                        ));
                    }
                }
            }
        }
        Err(err) => {
            match test.post.vector_post_value(vector).expect_exception {
                // Execution result was unsuccessful and an exception was expected.
                Some(expected_exceptions) => {
                    // Note: expected_exceptions is a vector because can only have 1 or 2 expected errors.
                    // So in exception_is_expected we find out if the obtained error matches one of the expected
                    if !exception_is_expected(expected_exceptions.clone(), err.clone()) {
                        let error_reason = match expected_exceptions.get(1) {
                            Some(second_exception) => {
                                format!(
                                    "Returned exception is not the expected: Returned {:?} but expected {:?} or {:?}",
                                    err,
                                    expected_exceptions.first().unwrap(),
                                    second_exception
                                )
                            }
                            None => {
                                format!(
                                    "Returned exception is not the expected: Returned {:?} but expected {:?}",
                                    err,
                                    expected_exceptions.first().unwrap()
                                )
                            }
                        };
                        return Err(EFTestRunnerError::ExpectedExceptionDoesNotMatchReceived(
                            format!("Post-state condition failed: {error_reason}"),
                        ));
                    }
                }
                // Execution result was unsuccessful but no exception was expected.
                None => {
                    return Err(EFTestRunnerError::ExecutionFailedUnexpectedly(err.clone()));
                }
            }
        }
    };
    Ok(())
}

pub fn get_state_transitions(
    initial_state: &EvmState,
    block_hash: H256,
    execution_report: &TransactionReport,
) -> Vec<AccountUpdate> {
    let current_db = match initial_state {
        EvmState::Store(state) => state.database.store.clone(),
        EvmState::Execution(_cache_db) => unreachable!("Execution state should not be passed here"),
    };
    let mut account_updates: Vec<AccountUpdate> = vec![];
    for (new_state_account_address, new_state_account) in &execution_report.new_state {
        let initial_account_state = current_db
            .get_account_info_by_hash(block_hash, *new_state_account_address)
            .expect("Error getting account info by address")
            .unwrap_or_default();
        let mut updates = 0;
        if initial_account_state.balance != new_state_account.info.balance {
            updates += 1;
        }
        if initial_account_state.nonce != new_state_account.info.nonce {
            updates += 1;
        }
        let code = if new_state_account.info.bytecode.is_empty() {
            // The new state account has no code
            None
        } else {
            // Get the code hash of the new state account bytecode
            let potential_new_bytecode_hash = code_hash(&new_state_account.info.bytecode);
            // Look into the current database to see if the bytecode hash is already present
            let current_bytecode = current_db
                .get_account_code(potential_new_bytecode_hash)
                .expect("Error getting account code by hash");
            let code = new_state_account.info.bytecode.clone();
            // The code is present in the current database
            if let Some(current_bytecode) = current_bytecode {
                if current_bytecode != code {
                    // The code has changed
                    Some(code)
                } else {
                    // The code has not changed
                    None
                }
            } else {
                // The new state account code is not present in the current
                // database, then it must be new
                Some(code)
            }
        };
        if code.is_some() {
            updates += 1;
        }
        let mut added_storage = HashMap::new();
        for (key, value) in &new_state_account.storage {
            added_storage.insert(*key, value.current_value);
            updates += 1;
        }
        if updates == 0 {
            continue;
        }

        let account_update = AccountUpdate {
            address: *new_state_account_address,
            removed: false,
            info: Some(AccountInfo {
                code_hash: code_hash(&new_state_account.info.bytecode),
                balance: new_state_account.info.balance,
                nonce: new_state_account.info.nonce,
            }),
            code,
            added_storage,
        };

        account_updates.push(account_update);
    }
    account_updates
}

pub fn post_state_root(account_updates: &[AccountUpdate], test: &EFTest) -> H256 {
    let (initial_state, block_hash) = utils::load_initial_state(test);
    initial_state
        .database()
        .unwrap()
        .apply_account_updates(block_hash, account_updates)
        .unwrap()
        .unwrap()
}
