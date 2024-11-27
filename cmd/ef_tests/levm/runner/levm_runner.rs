use crate::{
    report::{EFTestReport, TestVector},
    runner::{EFTestRunnerError, InternalError},
    types::EFTest,
    utils,
};
use ethrex_core::{
    types::{code_hash, AccountInfo},
    H256, U256,
};
use ethrex_levm::{
    db::Cache,
    errors::{TransactionReport, VMError},
    vm::VM,
    Environment,
};
use ethrex_storage::AccountUpdate;
use ethrex_vm::db::StoreWrapper;
use keccak_hash::keccak;
use std::{collections::HashMap, sync::Arc};

pub fn run_ef_test(test: &EFTest) -> Result<EFTestReport, EFTestRunnerError> {
    let mut ef_test_report = EFTestReport::new(
        test.name.clone(),
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
    VM::new(
        test.transactions.get(vector).unwrap().to.clone(),
        Environment {
            origin: test.transactions.get(vector).unwrap().sender,
            consumed_gas: U256::default(),
            refunded_gas: U256::default(),
            gas_limit: test
            .transactions
            .get(vector)
            .unwrap()
            .gas_limit,
            block_number: test.env.current_number,
            coinbase: test.env.current_coinbase,
            timestamp: test.env.current_timestamp,
            prev_randao: test.env.current_random,
            chain_id: U256::from(1729),
            base_fee_per_gas: test.env.current_base_fee.unwrap_or_default(),
            gas_price: test
                .transactions
                .get(vector)
                .unwrap()
                .gas_price
                .unwrap_or_default(), // or max_fee_per_gas?
            block_excess_blob_gas: test.env.current_excess_blob_gas,
            block_blob_gas_used: None,
            tx_blob_hashes: None,
        },
        test.transactions.get(vector).unwrap().value,
        test.transactions.get(vector).unwrap().data.clone(),
        db,
        Cache::default(),
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

pub fn ensure_post_state(
    levm_execution_result: &Result<TransactionReport, VMError>,
    vector: &TestVector,
    test: &EFTest,
) -> Result<(), EFTestRunnerError> {
    match levm_execution_result {
        Ok(execution_report) => {
            match test.post.vector_post_value(vector).expect_exception {
                // Execution result was successful but an exception was expected.
                Some(expected_exception) => {
                    let error_reason = format!("Expected exception: {expected_exception}");
                    return Err(EFTestRunnerError::FailedToEnsurePostState(
                        execution_report.clone(),
                        error_reason,
                    ));
                }
                // Execution result was successful and no exception was expected.
                None => {
                    let levm_account_updates = get_state_transitions(execution_report);
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
                // TODO: Check that the exception matches the expected exception.
                Some(_expected_exception) => {}
                // Execution result was unsuccessful but no exception was expected.
                None => {
                    return Err(EFTestRunnerError::ExecutionFailedUnexpectedly(err.clone()));
                }
            }
        }
    };
    Ok(())
}

pub fn get_state_transitions(execution_report: &TransactionReport) -> Vec<AccountUpdate> {
    let mut account_updates: Vec<AccountUpdate> = vec![];
    for (address, account) in &execution_report.new_state {
        let mut added_storage = HashMap::new();

        for (key, value) in &account.storage {
            added_storage.insert(*key, value.current_value);
        }

        let code = if account.info.bytecode.is_empty() {
            None
        } else {
            Some(account.info.bytecode.clone())
        };

        let account_update = AccountUpdate {
            address: *address,
            removed: false,
            info: Some(AccountInfo {
                code_hash: code_hash(&account.info.bytecode),
                balance: account.info.balance,
                nonce: account.info.nonce,
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
