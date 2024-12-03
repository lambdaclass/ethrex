use crate::{
    report::{ComparisonReport, EFTestReport, TestReRunReport, TestVector},
    runner::{
        levm_runner::{self, post_state_root},
        EFTestRunnerError, InternalError,
    },
    types::EFTest,
    utils::{effective_gas_price, load_initial_state},
};
use bytes::Bytes;
use ethrex_core::{types::TxKind, Address, H256};
use ethrex_levm::{
    errors::{TransactionReport, TxResult},
    Account, StorageSlot,
};
use ethrex_storage::{error::StoreError, AccountUpdate};
use ethrex_vm::{db::StoreWrapper, EvmState, RevmAddress, RevmU256, SpecId};
use revm::{
    db::State,
    inspectors::TracerEip3155 as RevmTracerEip3155,
    primitives::{
        AccessListItem, BlobExcessGasAndPrice, BlockEnv as RevmBlockEnv, EVMError as REVMError,
        ExecutionResult as RevmExecutionResult, TxEnv as RevmTxEnv, TxKind as RevmTxKind, B256,
    },
    Evm as Revm,
};
use std::collections::{HashMap, HashSet};

pub fn re_run_failed_ef_test(
    test: &EFTest,
    failed_test_report: &EFTestReport,
) -> Result<TestReRunReport, EFTestRunnerError> {
    assert_eq!(test.name, failed_test_report.name);
    let mut re_run_report = TestReRunReport::new();
    for (vector, vector_failure) in failed_test_report.failed_vectors.iter() {
        match vector_failure {
            // We only want to re-run tests that failed in the post-state validation.
            EFTestRunnerError::FailedToEnsurePostState(transaction_report, _) => {
                match re_run_failed_ef_test_tx(vector, test, transaction_report, &mut re_run_report) {
                    Ok(_) => continue,
                    Err(EFTestRunnerError::VMInitializationFailed(reason)) => {
                        return Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(
                            format!("REVM initialization failed when re-running failed test: {reason}"), re_run_report.clone()
                        )));
                    }
                    Err(EFTestRunnerError::Internal(reason)) => {
                        return Err(EFTestRunnerError::Internal(reason));
                    }
                    unexpected_error => {
                        return Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(format!(
                            "Unexpected error when re-running failed test: {unexpected_error:?}"
                        ), re_run_report.clone())));
                    }
                }
            },
            EFTestRunnerError::VMInitializationFailed(_)
            | EFTestRunnerError::ExecutionFailedUnexpectedly(_)
            | EFTestRunnerError::FailedToEnsurePreState(_) => continue,
            EFTestRunnerError::VMExecutionMismatch(reason) => return Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(
                format!("VM execution mismatch errors should only happen when running with revm. This failed during levm's execution: {reason}"), re_run_report.clone()))),
            EFTestRunnerError::Internal(reason) => return Err(EFTestRunnerError::Internal(reason.to_owned())),
        }
    }
    Ok(re_run_report)
}

pub fn re_run_failed_ef_test_tx(
    vector: &TestVector,
    test: &EFTest,
    levm_execution_report: &TransactionReport,
    re_run_report: &mut TestReRunReport,
) -> Result<(), EFTestRunnerError> {
    let (mut state, _block_hash) = load_initial_state(test);
    let mut revm = prepare_revm_for_tx(&mut state, vector, test)?;
    let revm_execution_result = revm.transact_commit();
    drop(revm); // Need to drop the state mutable reference.
    compare_levm_revm_execution_results(
        vector,
        levm_execution_report,
        revm_execution_result,
        re_run_report,
    )?;
    ensure_post_state(
        levm_execution_report,
        vector,
        &mut state,
        test,
        re_run_report,
    )?;
    Ok(())
}

pub fn prepare_revm_for_tx<'state>(
    initial_state: &'state mut EvmState,
    vector: &TestVector,
    test: &EFTest,
) -> Result<Revm<'state, RevmTracerEip3155, &'state mut State<StoreWrapper>>, EFTestRunnerError> {
    let chain_spec = initial_state
        .chain_config()
        .map_err(|err| EFTestRunnerError::VMInitializationFailed(err.to_string()))?;
    let block_env = RevmBlockEnv {
        number: RevmU256::from_limbs(test.env.current_number.0),
        coinbase: RevmAddress(test.env.current_coinbase.0.into()),
        timestamp: RevmU256::from_limbs(test.env.current_timestamp.0),
        gas_limit: RevmU256::from_limbs(test.env.current_gas_limit.0),
        basefee: RevmU256::from_limbs(test.env.current_base_fee.unwrap_or_default().0),
        difficulty: RevmU256::from_limbs(test.env.current_difficulty.0),
        prevrandao: test.env.current_random.map(|v| v.0.into()),
        blob_excess_gas_and_price: test
            .env
            .current_excess_blob_gas
            .map(|gas| BlobExcessGasAndPrice::new(gas.as_u64())),
    };
    let tx = &test
        .transactions
        .get(vector)
        .ok_or(EFTestRunnerError::VMInitializationFailed(format!(
            "Vector {vector:?} not found in test {}",
            test.name
        )))?;

    let revm_access_list: Vec<AccessListItem> = tx
        .access_list
        .iter()
        .map(|eftest_access_list_item| AccessListItem {
            address: RevmAddress(eftest_access_list_item.address.0.into()),
            storage_keys: eftest_access_list_item
                .storage_keys
                .iter()
                .map(|key| B256::from(key.0))
                .collect(),
        })
        .collect();

    let tx_env = RevmTxEnv {
        caller: tx.sender.0.into(),
        gas_limit: tx.gas_limit.as_u64(),
        gas_price: RevmU256::from_limbs(effective_gas_price(test, tx)?.0),
        transact_to: match tx.to {
            TxKind::Call(to) => RevmTxKind::Call(to.0.into()),
            TxKind::Create => RevmTxKind::Create,
        },
        value: RevmU256::from_limbs(tx.value.0),
        data: tx.data.to_vec().into(),
        nonce: Some(tx.nonce.as_u64()),
        chain_id: Some(chain_spec.chain_id), //TODO: See what to do with this... ChainId test fails IDK why.
        access_list: revm_access_list,
        gas_priority_fee: tx
            .max_priority_fee_per_gas
            .map(|fee| RevmU256::from_limbs(fee.0)),
        blob_hashes: tx
            .blob_versioned_hashes
            .iter()
            .map(|h256| B256::from(h256.0))
            .collect::<Vec<B256>>(),
        max_fee_per_blob_gas: tx
            .max_fee_per_blob_gas
            .map(|fee| RevmU256::from_limbs(fee.0)),
        authorization_list: None,
    };

    let evm_builder = Revm::builder()
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain_id)
        .with_spec_id(SpecId::CANCUN) //TODO: In the future replace cancun for the actual spec id
        .with_external_context(
            RevmTracerEip3155::new(Box::new(std::io::stderr())).without_summary(),
        );
    match initial_state {
        EvmState::Store(db) => Ok(evm_builder.with_db(db).build()),
        _ => Err(EFTestRunnerError::VMInitializationFailed(
            "Expected LEVM state to be a Store".to_owned(),
        )),
    }
}

pub fn compare_levm_revm_execution_results(
    vector: &TestVector,
    levm_execution_report: &TransactionReport,
    revm_execution_result: Result<RevmExecutionResult, REVMError<StoreError>>,
    re_run_report: &mut TestReRunReport,
) -> Result<(), EFTestRunnerError> {
    match (levm_execution_report, revm_execution_result) {
        (levm_tx_report, Ok(revm_execution_result)) => {
            match (&levm_tx_report.result, revm_execution_result.clone()) {
                (
                    TxResult::Success,
                    RevmExecutionResult::Success {
                        reason: _,
                        gas_used: revm_gas_used,
                        gas_refunded: revm_gas_refunded,
                        logs: _,
                        output: _,
                    },
                ) => {
                    if levm_tx_report.gas_used != revm_gas_used {
                        re_run_report.register_gas_used_mismatch(
                            *vector,
                            levm_tx_report.gas_used,
                            revm_gas_used,
                        );
                    }
                    if levm_tx_report.gas_refunded != revm_gas_refunded {
                        re_run_report.register_gas_refunded_mismatch(
                            *vector,
                            levm_tx_report.gas_refunded,
                            revm_gas_refunded,
                        );
                    }
                }
                (
                    TxResult::Revert(_error),
                    RevmExecutionResult::Revert {
                        gas_used: revm_gas_used,
                        output: _,
                    },
                ) => {
                    if levm_tx_report.gas_used != revm_gas_used {
                        re_run_report.register_gas_used_mismatch(
                            *vector,
                            levm_tx_report.gas_used,
                            revm_gas_used,
                        );
                    }
                }
                (
                    TxResult::Revert(_error),
                    RevmExecutionResult::Halt {
                        reason: _,
                        gas_used: revm_gas_used,
                    },
                ) => {
                    // TODO: Register the revert reasons.
                    if levm_tx_report.gas_used != revm_gas_used {
                        re_run_report.register_gas_used_mismatch(
                            *vector,
                            levm_tx_report.gas_used,
                            revm_gas_used,
                        );
                    }
                }
                _ => {
                    re_run_report.register_execution_result_mismatch(
                        *vector,
                        levm_tx_report.result.clone(),
                        revm_execution_result.clone(),
                    );
                }
            }
        }
        (levm_transaction_report, Err(revm_error)) => {
            re_run_report.register_re_run_failure(
                *vector,
                levm_transaction_report.result.clone(),
                revm_error,
            );
        }
    }
    Ok(())
}

pub fn ensure_post_state(
    levm_execution_report: &TransactionReport,
    vector: &TestVector,
    revm_state: &mut EvmState,
    test: &EFTest,
    re_run_report: &mut TestReRunReport,
) -> Result<(), EFTestRunnerError> {
    match test.post.vector_post_value(vector).expect_exception {
        Some(_expected_exception) => {}
        // We only want to compare account updates when no exception is expected.
        None => {
            let (initial_state, _block_hash) = load_initial_state(test);
            let levm_account_updates =
                levm_runner::get_state_transitions(&initial_state, levm_execution_report);
            let revm_account_updates = ethrex_vm::get_state_transitions(revm_state);
            let account_updates_report = compare_levm_revm_account_updates(
                test,
                &levm_account_updates,
                &revm_account_updates,
            );
            re_run_report.register_account_updates_report(*vector, account_updates_report);
        }
    }

    Ok(())
}

pub fn compare_levm_revm_account_updates(
    test: &EFTest,
    levm_account_updates: &[AccountUpdate],
    revm_account_updates: &[AccountUpdate],
) -> ComparisonReport {
    let levm_post_state_root = post_state_root(levm_account_updates, test);
    let revm_post_state_root = post_state_root(revm_account_updates, test);
    let mut initial_accounts: HashMap<Address, Account> = test
        .pre
        .0
        .iter()
        .map(|(account_address, pre_state_value)| {
            let account_storage = pre_state_value
                .storage
                .iter()
                .map(|(key, value)| {
                    let mut temp = [0u8; 32];
                    key.to_big_endian(&mut temp);
                    let storage_slot = StorageSlot {
                        original_value: *value,
                        current_value: *value,
                    };
                    (H256::from_slice(&temp), storage_slot)
                })
                .collect();
            let account = Account::new(
                pre_state_value.balance,
                pre_state_value.code.clone(),
                pre_state_value.nonce.as_u64(),
                account_storage,
            );
            (*account_address, account)
        })
        .collect();
    initial_accounts
        .entry(test.env.current_coinbase)
        .or_default();
    let levm_updated_accounts = levm_account_updates
        .iter()
        .map(|account_update| account_update.address)
        .collect::<HashSet<Address>>();
    let revm_updated_accounts = revm_account_updates
        .iter()
        .map(|account_update| account_update.address)
        .collect::<HashSet<Address>>();

    ComparisonReport {
        levm_post_state_root,
        revm_post_state_root,
        initial_accounts,
        levm_account_updates: levm_account_updates.to_vec(),
        revm_account_updates: revm_account_updates.to_vec(),
        levm_updated_accounts_only: levm_updated_accounts
            .difference(&revm_updated_accounts)
            .cloned()
            .collect::<HashSet<Address>>(),
        revm_updated_accounts_only: revm_updated_accounts
            .difference(&levm_updated_accounts)
            .cloned()
            .collect::<HashSet<Address>>(),
        shared_updated_accounts: levm_updated_accounts
            .intersection(&revm_updated_accounts)
            .cloned()
            .collect::<HashSet<Address>>(),
    }
}

pub fn _run_ef_test_revm(test: &EFTest) -> Result<EFTestReport, EFTestRunnerError> {
    dbg!(&test.name);
    let mut ef_test_report = EFTestReport::new(
        test.name.clone(),
        test.dir.clone(),
        test._info.generated_test_hash,
        test.fork(),
    );
    for (vector, _tx) in test.transactions.iter() {
        match _run_ef_test_tx_revm(vector, test) {
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
                    "VM execution mismatch errors should only happen when COMPARING LEVM AND REVM. This failed during revm's execution."
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

pub fn _run_ef_test_tx_revm(vector: &TestVector, test: &EFTest) -> Result<(), EFTestRunnerError> {
    // dbg!(vector);
    let (mut state, _block_hash) = load_initial_state(test);
    let mut revm = prepare_revm_for_tx(&mut state, vector, test)?;
    let revm_execution_result = revm.transact_commit();
    drop(revm); // Need to drop the state mutable reference.

    _ensure_post_state_revm(revm_execution_result, vector, test, &mut state)?;

    Ok(())
}

pub fn _ensure_post_state_revm(
    revm_execution_result: Result<RevmExecutionResult, REVMError<StoreError>>,
    vector: &TestVector,
    test: &EFTest,
    revm_state: &mut EvmState,
) -> Result<(), EFTestRunnerError> {
    match revm_execution_result {
        Ok(_execution_result) => {
            match test.post.vector_post_value(vector).expect_exception {
                // Execution result was successful but an exception was expected.
                Some(expected_exception) => {
                    let error_reason = format!("Expected exception: {expected_exception}");
                    println!("Expected exception: {expected_exception}");
                    return Err(EFTestRunnerError::FailedToEnsurePostState(
                        TransactionReport {
                            result: TxResult::Success,
                            gas_used: 42,
                            gas_refunded: 42,
                            logs: vec![],
                            output: Bytes::new(),
                            new_state: HashMap::new(),
                            created_address: None,
                        },
                        //TODO: This is not a TransactionReport because it is REVM
                        error_reason,
                    ));
                }
                // Execution result was successful and no exception was expected.
                None => {
                    let revm_account_updates = ethrex_vm::get_state_transitions(revm_state);
                    let pos_state_root = post_state_root(&revm_account_updates, test);
                    let expected_post_state_root_hash = test.post.vector_post_value(vector).hash;
                    if expected_post_state_root_hash != pos_state_root {
                        println!(
                            "Post-state root mismatch: expected {expected_post_state_root_hash:#x}, got {pos_state_root:#x}",
                        );
                        let error_reason = format!(
                            "Post-state root mismatch: expected {expected_post_state_root_hash:#x}, got {pos_state_root:#x}",
                        );
                        return Err(EFTestRunnerError::FailedToEnsurePostState(
                            TransactionReport {
                                result: TxResult::Success,
                                gas_used: 42,
                                gas_refunded: 42,
                                logs: vec![],
                                output: Bytes::new(),
                                new_state: HashMap::new(),
                                created_address: None,
                            },
                            //TODO: This is not a TransactionReport because it is REVM
                            error_reason,
                        ));
                    }
                }
            }
        }
        Err(err) => {
            match test.post.vector_post_value(vector).expect_exception {
                // Execution result was unsuccessful and an exception was expected.
                // TODO: See if we want to map revm exceptions to expected exceptions, probably not.
                Some(_expected_exception) => {}
                // Execution result was unsuccessful but no exception was expected.
                None => {
                    println!(
                        "Unexpected exception. Name: {}, vector: {:?}, error: {:?}",
                        &test.name, vector, err
                    );
                    return Err(EFTestRunnerError::ExecutionFailedUnexpectedly(
                        ethrex_levm::errors::VMError::AddressAlreadyOccupied,
                        //TODO: Use another kind of error for this.
                    ));
                }
            }
        }
    };
    Ok(())
}
