use crate::{
    report::{EFTestReport, EFTestReportForkResult, TestVector},
    runner::{EFTestRunnerError, InternalError},
    types::{EFTest, TransactionExpectedException},
    utils::{self, effective_gas_price},
};
use ethrex_common::{
    types::{tx_fields::*, Fork},
    H256, U256,
};
use ethrex_levm::{
    db::CacheDB,
    errors::{ExecutionReport, TxValidationError, VMError},
    vm::{EVMConfig, VM},
    Environment,
};
use ethrex_storage::AccountUpdate;
use ethrex_vm::{
    backends::{self},
    db::StoreWrapper,
};
use keccak_hash::keccak;
use std::{collections::HashMap, sync::Arc};

pub fn run_ef_test(test: &EFTest) -> Result<EFTestReport, EFTestRunnerError> {
    // There are some tests that don't have a hash, unwrap will panic
    let hash = test
        ._info
        .generated_test_hash
        .or(test._info.hash)
        .unwrap_or_default();

    let mut ef_test_report = EFTestReport::new(test.name.clone(), test.dir.clone(), hash);
    for fork in test.post.forks.keys() {
        let mut ef_test_report_fork = EFTestReportForkResult::new();

        for (vector, _tx) in test.transactions.iter() {
            // This is because there are some test vectors that are not valid for the current fork.
            if !test.post.has_vector_for_fork(vector, *fork) {
                continue;
            }
            match run_ef_test_tx(vector, test, fork) {
                Ok(_) => continue,
                Err(EFTestRunnerError::VMInitializationFailed(reason)) => {
                    ef_test_report_fork.register_vm_initialization_failure(reason, *vector);
                }
                Err(EFTestRunnerError::FailedToEnsurePreState(reason)) => {
                    ef_test_report_fork.register_pre_state_validation_failure(reason, *vector);
                }
                Err(EFTestRunnerError::ExecutionFailedUnexpectedly(error)) => {
                    ef_test_report_fork.register_unexpected_execution_failure(error, *vector);
                }
                Err(EFTestRunnerError::FailedToEnsurePostState(transaction_report, reason)) => {
                    ef_test_report_fork.register_post_state_validation_failure(
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
                    ef_test_report_fork
                        .register_post_state_validation_error_mismatch(reason, *vector);
                }
                Err(EFTestRunnerError::Internal(reason)) => {
                    return Err(EFTestRunnerError::Internal(reason));
                }
            }
        }
        ef_test_report.register_fork_result(*fork, ef_test_report_fork);
    }
    Ok(ef_test_report)
}

pub fn run_ef_test_tx(
    vector: &TestVector,
    test: &EFTest,
    fork: &Fork,
) -> Result<(), EFTestRunnerError> {
    let mut levm = prepare_vm_for_tx(vector, test, fork)?;
    ensure_pre_state(&levm, test)?;
    let levm_execution_result = levm.execute();
    ensure_post_state(&levm_execution_result, vector, test, fork)?;
    Ok(())
}

pub fn prepare_vm_for_tx(
    vector: &TestVector,
    test: &EFTest,
    fork: &Fork,
) -> Result<VM, EFTestRunnerError> {
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

    // Check if the tx has the authorization_lists field implemented by eip7702.
    let authorization_list = tx.authorization_list.clone().map(|list| {
        list.iter()
            .map(|auth_tuple| AuthorizationTuple {
                chain_id: auth_tuple.chain_id,
                address: auth_tuple.address,
                nonce: auth_tuple.nonce,
                y_parity: auth_tuple.v,
                r_signature: auth_tuple.r,
                s_signature: auth_tuple.s,
            })
            .collect::<Vec<AuthorizationTuple>>()
    });

    let blob_schedule = EVMConfig::canonical_values(*fork);
    let config = EVMConfig::new(*fork, blob_schedule);

    VM::new(
        tx.to.clone(),
        Environment {
            origin: tx.sender,
            refunded_gas: 0,
            gas_limit: tx.gas_limit,
            config,
            block_number: test.env.current_number,
            coinbase: test.env.current_coinbase,
            timestamp: test.env.current_timestamp,
            prev_randao: test.env.current_random,
            difficulty: test.env.current_difficulty,
            chain_id: U256::from(1),
            base_fee_per_gas: test.env.current_base_fee.unwrap_or_default(),
            gas_price: effective_gas_price(test, &tx)?,
            block_excess_blob_gas: test.env.current_excess_blob_gas,
            block_blob_gas_used: None,
            tx_blob_hashes: tx.blob_versioned_hashes.clone(),
            tx_max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
            tx_max_fee_per_gas: tx.max_fee_per_gas,
            tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas,
            tx_nonce: tx.nonce.try_into().map_err(|_| {
                EFTestRunnerError::VMInitializationFailed("Nonce to large".to_string())
            })?,
            block_gas_limit: test.env.current_gas_limit,
            transient_storage: HashMap::new(),
        },
        tx.value,
        tx.data.clone(),
        db,
        CacheDB::default(),
        access_lists,
        authorization_list,
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
            let storage_slot =
                world_state.get_storage_slot(*address, H256::from_slice(&k.to_big_endian()));
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

// Exceptions not covered: RlpInvalidValue
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
                TransactionExpectedException::Type3TxPreFork,
                VMError::TxValidation(TxValidationError::Type3TxPreFork)
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
            ) | (
                TransactionExpectedException::Type4TxContractCreation,
                VMError::TxValidation(TxValidationError::Type4TxContractCreation)
            )
        )
    })
}

pub fn ensure_post_state(
    levm_execution_result: &Result<ExecutionReport, VMError>,
    vector: &TestVector,
    test: &EFTest,
    fork: &Fork,
) -> Result<(), EFTestRunnerError> {
    match levm_execution_result {
        Ok(execution_report) => {
            match test.post.vector_post_value(vector, *fork).expect_exception {
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
                    let (initial_state, block_hash) = utils::load_initial_state(test); //TODO: Stop using EvmState in LEVM
                    let levm_account_updates = backends::levm::LEVM::get_state_transitions(
                        Some(*fork),
                        &initial_state.database().unwrap(),
                        block_hash,
                        &execution_report.new_state,
                    )
                    .map_err(|_| {
                        InternalError::Custom(
                            "Error at LEVM::get_state_transitions in ensure_post_state()"
                                .to_owned(),
                        )
                    })?;
                    let pos_state_root = post_state_root(&levm_account_updates, test);
                    let expected_post_state_root_hash =
                        test.post.vector_post_value(vector, *fork).hash;
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
            match test.post.vector_post_value(vector, *fork).expect_exception {
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

pub fn post_state_root(account_updates: &[AccountUpdate], test: &EFTest) -> H256 {
    let (initial_state, block_hash) = utils::load_initial_state(test);
    initial_state
        .database()
        .unwrap()
        .apply_account_updates(block_hash, account_updates)
        .unwrap()
        .unwrap()
}
