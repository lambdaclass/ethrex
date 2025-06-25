use crate::{
    report::{ComparisonReport, EFTestReport, EFTestReportForkResult, TestReRunReport, TestVector},
    runner::{EFTestRunnerError, InternalError, levm_runner::post_state_root},
    types::EFTest,
    utils::{effective_gas_price, load_initial_state, load_initial_state_levm},
};
use alloy_rlp::Encodable;
use bytes::Bytes;
use ethrex_common::{
    Address, H256,
    types::{Account, AccountUpdate, Fork, TxKind},
};
use ethrex_levm::errors::{ExecutionReport, TxResult};
use ethrex_rlp::encode::RLPEncode;
use ethrex_vm::{
    self, DynVmDatabase, EvmError,
    backends::{self, revm::db::EvmState},
    fork_to_spec_id,
};
use keccak_hash::keccak;
pub use revm::primitives::{Address as RevmAddress, SpecId, U256 as RevmU256};
use revm::{
    Evm as Revm,
    db::State,
    inspectors::TracerEip3155 as RevmTracerEip3155,
    primitives::{
        AccessListItem, Authorization, B256, BlobExcessGasAndPrice, BlockEnv as RevmBlockEnv,
        EVMError as REVMError, ExecutionResult as RevmExecutionResult, SignedAuthorization,
        TxEnv as RevmTxEnv, TxKind as RevmTxKind,
    },
};
use std::collections::{HashMap, HashSet};

fn levm_and_revm_logs_match(
    levm_logs: &Vec<ethrex_common::types::Log>,
    revm_logs: &Vec<revm::primitives::Log>,
) -> bool {
    let levm_keccak_logs = {
        let logs = levm_logs;
        let mut encoded_logs = Vec::new();
        logs.encode(&mut encoded_logs);
        keccak(encoded_logs)
    };

    let revm_keccak_logs = {
        let logs = revm_logs;
        let mut encoded_logs = Vec::new();
        logs.encode(&mut encoded_logs);
        keccak(encoded_logs)
    };

    levm_keccak_logs == revm_keccak_logs
}

pub async fn re_run_failed_ef_test(
    test: &EFTest,
    failed_test_report: &EFTestReport,
) -> Result<TestReRunReport, EFTestRunnerError> {
    assert_eq!(test.name, failed_test_report.name);
    let mut re_run_report = TestReRunReport::new();
    for (fork, fork_result) in failed_test_report.fork_results.iter() {
        for (vector, vector_failure) in fork_result.failed_vectors.iter() {
            match vector_failure {
                // We only want to re-run tests that failed in the post-state validation.
                EFTestRunnerError::FailedToEnsurePostState(transaction_report, _, levm_cache) => {
                    match re_run_failed_ef_test_tx(
                        levm_cache.clone(),
                        vector,
                        test,
                        transaction_report,
                        &mut re_run_report,
                        fork,
                    )
                    .await
                    {
                        Ok(_) => continue,
                        Err(EFTestRunnerError::VMInitializationFailed(reason)) => {
                            return Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(
                                format!(
                                    "REVM initialization failed when re-running failed test: {reason}"
                                ),
                                re_run_report.clone(),
                            )));
                        }
                        Err(EFTestRunnerError::Internal(reason)) => {
                            return Err(EFTestRunnerError::Internal(reason));
                        }
                        unexpected_error => {
                            return Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(
                                format!(
                                    "Unexpected error when re-running failed test: {unexpected_error:?}"
                                ),
                                re_run_report.clone(),
                            )));
                        }
                    }
                }
                // Currently, we decided not to re-execute the test when the Expected exception does not match
                // with the received. This can change in the future.
                EFTestRunnerError::ExpectedExceptionDoesNotMatchReceived(_) => continue,
                EFTestRunnerError::VMInitializationFailed(_)
                | EFTestRunnerError::ExecutionFailedUnexpectedly(_)
                | EFTestRunnerError::FailedToEnsurePreState(_)
                | EFTestRunnerError::EIP7702ShouldNotBeCreateType
                | EFTestRunnerError::FailedToRevertLEVMState(_) => continue,
                EFTestRunnerError::VMExecutionMismatch(reason) => {
                    return Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(
                        format!(
                            "VM execution mismatch errors should only happen when running with revm. This failed during levm's execution: {reason}"
                        ),
                        re_run_report.clone(),
                    )));
                }
                EFTestRunnerError::Internal(reason) => {
                    return Err(EFTestRunnerError::Internal(reason.to_owned()));
                }
                EFTestRunnerError::TestsFailed => {
                    unreachable!(
                        "An EFTestRunnerError::TestsFailed can't happen at this point. This error is only thrown in run_ef_tests under the summary flag"
                    )
                }
            }
        }
    }
    Ok(re_run_report)
}

pub async fn re_run_failed_ef_test_tx(
    levm_cache: HashMap<Address, Account>,
    vector: &TestVector,
    test: &EFTest,
    levm_execution_report: &ExecutionReport,
    re_run_report: &mut TestReRunReport,
    fork: &Fork,
) -> Result<(), EFTestRunnerError> {
    let (mut state, _block_hash, _store) = load_initial_state(test).await;
    let mut revm = prepare_revm_for_tx(&mut state, vector, test, fork)?;
    if !test.post.has_vector_for_fork(vector, *fork) {
        return Ok(());
    }
    let revm_execution_result = revm.transact_commit();
    drop(revm); // Need to drop the state mutable reference.
    compare_levm_revm_execution_results(
        vector,
        levm_execution_report,
        revm_execution_result,
        re_run_report,
        fork,
    )?;
    ensure_post_state(levm_cache, vector, &mut state, test, re_run_report, fork).await?;
    Ok(())
}

pub fn prepare_revm_for_tx<'state>(
    initial_state: &'state mut EvmState,
    vector: &TestVector,
    test: &EFTest,
    fork: &Fork,
) -> Result<Revm<'state, RevmTracerEip3155, &'state mut State<DynVmDatabase>>, EFTestRunnerError> {
    let chain_spec = initial_state
        .chain_config()
        .map_err(|err| EFTestRunnerError::VMInitializationFailed(err.to_string()))?;

    let blob_excess_gas_and_price = if test.env.current_excess_blob_gas.is_none() {
        None
    } else {
        Some(BlobExcessGasAndPrice::new(
            test.env.current_excess_blob_gas.unwrap().as_u64(),
            *fork == Fork::Prague,
        ))
    };
    let block_env = RevmBlockEnv {
        number: RevmU256::from_limbs(test.env.current_number.0),
        coinbase: RevmAddress(test.env.current_coinbase.0.into()),
        timestamp: RevmU256::from_limbs(test.env.current_timestamp.0),
        gas_limit: RevmU256::from(test.env.current_gas_limit),
        basefee: RevmU256::from_limbs(test.env.current_base_fee.unwrap_or_default().0),
        difficulty: RevmU256::from_limbs(test.env.current_difficulty.0),
        prevrandao: test.env.current_random.map(|v| v.0.into()),
        blob_excess_gas_and_price,
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

    // The latest version of revm(19.3.0) is needed to run the ef-tests with the latest changes.
    // Update it in every Cargo.toml.
    // revm-inspectors and revm-primitives have to be bumped too.
    // NOTE:
    // - rust 1.82.X is needed
    // - rust-toolchain 1.82.X is needed (this can be found in ethrex/crates/vm/levm/rust-toolchain.toml)
    let authorization_list = tx.authorization_list.clone().map(|list| {
        list.iter()
            .map(|auth_t| {
                SignedAuthorization::new_unchecked(
                    Authorization {
                        // The latest spec defined chain_id as a U256
                        chain_id: RevmU256::from_le_bytes(auth_t.chain_id.to_little_endian()),
                        address: RevmAddress(auth_t.address.0.into()),
                        nonce: auth_t.nonce,
                    },
                    auth_t.v.as_u32() as u8,
                    RevmU256::from_le_bytes(auth_t.r.to_little_endian()),
                    RevmU256::from_le_bytes(auth_t.s.to_little_endian()),
                )
            })
            .collect::<Vec<SignedAuthorization>>()
            .into()
    });

    let tx_env = RevmTxEnv {
        caller: tx.sender.0.into(),
        gas_limit: tx.gas_limit,
        gas_price: RevmU256::from_limbs(effective_gas_price(test, tx)?.0),
        transact_to: match tx.to {
            TxKind::Call(to) => RevmTxKind::Call(to.0.into()),
            TxKind::Create => RevmTxKind::Create,
        },
        value: RevmU256::from_limbs(tx.value.0),
        data: tx.data.to_vec().into(),
        nonce: Some(tx.nonce),
        chain_id: Some(chain_spec.chain_id),
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
        authorization_list,
    };

    let evm_builder = Revm::builder()
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain_id)
        .with_spec_id(fork_to_spec_id(*fork))
        .with_external_context(
            RevmTracerEip3155::new(Box::new(std::io::stderr())).without_summary(),
        );
    Ok(evm_builder.with_db(&mut initial_state.inner).build())
}

pub fn compare_levm_revm_execution_results(
    vector: &TestVector,
    levm_execution_report: &ExecutionReport,
    revm_execution_result: Result<RevmExecutionResult, REVMError<EvmError>>,
    re_run_report: &mut TestReRunReport,
    fork: &Fork,
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
                        logs: revm_logs,
                        output: _,
                    },
                ) => {
                    if levm_tx_report.gas_used != revm_gas_used {
                        re_run_report.register_gas_used_mismatch(
                            *vector,
                            levm_tx_report.gas_used,
                            revm_gas_used,
                            *fork,
                        );
                    }
                    if levm_tx_report.gas_refunded != revm_gas_refunded {
                        re_run_report.register_gas_refunded_mismatch(
                            *vector,
                            levm_tx_report.gas_refunded,
                            revm_gas_refunded,
                            *fork,
                        );
                    }

                    if !levm_and_revm_logs_match(&levm_tx_report.logs, &revm_logs) {
                        re_run_report.register_logs_mismatch(
                            *vector,
                            levm_tx_report.logs.clone(),
                            revm_logs.clone(),
                            *fork,
                        )
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
                            *fork,
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
                            *fork,
                        );
                    }
                }
                _ => {
                    re_run_report.register_execution_result_mismatch(
                        *vector,
                        levm_tx_report.result.clone(),
                        revm_execution_result.clone(),
                        *fork,
                    );
                }
            }
        }
        (levm_transaction_report, Err(revm_error)) => {
            re_run_report.register_re_run_failure(
                *vector,
                levm_transaction_report.result.clone(),
                revm_error,
                *fork,
            );
        }
    }
    Ok(())
}

pub async fn ensure_post_state(
    levm_cache: HashMap<Address, Account>,
    vector: &TestVector,
    revm_state: &mut EvmState,
    test: &EFTest,
    re_run_report: &mut TestReRunReport,
    fork: &Fork,
) -> Result<(), EFTestRunnerError> {
    match test.post.vector_post_value(vector, *fork).expect_exception {
        Some(_expected_exception) => {}
        // We only want to compare account updates when no exception is expected.
        None => {
            let mut db = load_initial_state_levm(test).await;
            db.current_accounts_state = levm_cache;
            let levm_account_updates = backends::levm::LEVM::get_state_transitions(&mut db)
                .map_err(|_| {
                    InternalError::Custom("Error at LEVM::get_state_transitions()".to_owned())
                })?;
            let revm_account_updates = backends::revm::REVM::get_state_transitions(revm_state);
            let account_updates_report = compare_levm_revm_account_updates(
                vector,
                test,
                fork,
                &levm_account_updates,
                &revm_account_updates,
            )
            .await;
            re_run_report.register_account_updates_report(*vector, account_updates_report, *fork);
        }
    }

    Ok(())
}

pub async fn compare_levm_revm_account_updates(
    vector: &TestVector,
    test: &EFTest,
    fork: &Fork,
    levm_account_updates: &[AccountUpdate],
    revm_account_updates: &[AccountUpdate],
) -> ComparisonReport {
    let levm_post_state_root = post_state_root(levm_account_updates, test).await;
    let revm_post_state_root = post_state_root(revm_account_updates, test).await;
    let mut initial_accounts: HashMap<Address, Account> = test
        .pre
        .0
        .iter()
        .map(|(account_address, pre_state_value)| {
            let account_storage = pre_state_value
                .storage
                .iter()
                .map(|(key, value)| (H256::from_slice(&key.to_big_endian()), *value))
                .collect();
            let account = Account::new(
                pre_state_value.balance,
                pre_state_value.code.clone(),
                pre_state_value.nonce,
                account_storage,
            );
            (*account_address, account)
        })
        .collect();
    initial_accounts
        .entry(test.env.current_coinbase)
        .or_default();

    let (levm_updated_accounts, revm_updated_accounts): (HashSet<_>, HashSet<_>) = (
        levm_account_updates.iter().map(|u| u.address).collect(),
        revm_account_updates.iter().map(|u| u.address).collect(),
    );

    ComparisonReport {
        levm_post_state_root,
        revm_post_state_root,
        initial_accounts,
        expected_post_state_root: test.post.vector_post_value(vector, *fork).hash,
        levm_account_updates: levm_account_updates.to_vec(),
        revm_account_updates: revm_account_updates.to_vec(),
        levm_updated_accounts_only: &levm_updated_accounts - &revm_updated_accounts,
        revm_updated_accounts_only: &revm_updated_accounts - &levm_updated_accounts,
        shared_updated_accounts: &levm_updated_accounts & &revm_updated_accounts,
    }
}

pub async fn _run_ef_test_revm(test: &EFTest) -> Result<EFTestReport, EFTestRunnerError> {
    let hash = test
        ._info
        .generated_test_hash
        .or(test._info.hash)
        .unwrap_or_default();

    let mut ef_test_report = EFTestReport::new(test.name.clone(), test.dir.clone(), hash);
    for fork in test.post.forks.keys() {
        let mut ef_test_report_fork = EFTestReportForkResult::new();

        for (vector, _tx) in test.transactions.iter() {
            if !test.post.has_vector_for_fork(vector, *fork) {
                continue;
            }
            match _run_ef_test_tx_revm(vector, test, fork).await {
                Ok(_) => continue,
                Err(EFTestRunnerError::VMInitializationFailed(reason)) => {
                    ef_test_report_fork.register_vm_initialization_failure(reason, *vector);
                }
                Err(EFTestRunnerError::FailedToEnsurePreState(reason))
                | Err(EFTestRunnerError::FailedToRevertLEVMState(reason)) => {
                    ef_test_report_fork.register_pre_state_validation_failure(reason, *vector);
                }
                Err(EFTestRunnerError::ExecutionFailedUnexpectedly(error)) => {
                    ef_test_report_fork.register_unexpected_execution_failure(error, *vector);
                }
                Err(EFTestRunnerError::FailedToEnsurePostState(
                    transaction_report,
                    reason,
                    levm_cache,
                )) => {
                    ef_test_report_fork.register_post_state_validation_failure(
                        *transaction_report,
                        reason,
                        *vector,
                        levm_cache,
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
                Err(EFTestRunnerError::ExpectedExceptionDoesNotMatchReceived(_)) => {
                    return Err(EFTestRunnerError::Internal(InternalError::MainRunnerInternal(
                        "The ExpectedExceptionDoesNotMatchReceived error should only happen when executing Levm, the errors matching is not implemented in Revm"
                            .to_owned(),
                    )));
                }
                Err(EFTestRunnerError::EIP7702ShouldNotBeCreateType) => {
                    return Err(EFTestRunnerError::Internal(InternalError::Custom(
                        "This case should not happen".to_owned(),
                    )));
                }
                Err(EFTestRunnerError::TestsFailed) => {
                    unreachable!(
                        "An EFTestRunnerError::TestsFailed can't happen at this point. This error is only thrown in run_ef_tests under the summary flag"
                    )
                }
            }
        }
        ef_test_report.register_fork_result(*fork, ef_test_report_fork);
    }
    Ok(ef_test_report)
}

pub async fn _run_ef_test_tx_revm(
    vector: &TestVector,
    test: &EFTest,
    fork: &Fork,
) -> Result<(), EFTestRunnerError> {
    let (mut state, _block_hash, _store) = load_initial_state(test).await;
    let mut revm = prepare_revm_for_tx(&mut state, vector, test, fork)?;
    let revm_execution_result = revm.transact_commit();
    drop(revm); // Need to drop the state mutable reference.

    _ensure_post_state_revm(revm_execution_result, vector, test, &mut state, fork).await?;

    Ok(())
}

pub async fn _ensure_post_state_revm(
    revm_execution_result: Result<RevmExecutionResult, REVMError<EvmError>>,
    vector: &TestVector,
    test: &EFTest,
    revm_state: &mut EvmState,
    fork: &Fork,
) -> Result<(), EFTestRunnerError> {
    match revm_execution_result {
        Ok(_execution_result) => {
            match test.post.vector_post_value(vector, *fork).expect_exception {
                // Execution result was successful but an exception was expected.
                Some(expected_exception) => {
                    let error_reason = format!("Expected exception: {expected_exception:?}");
                    return Err(EFTestRunnerError::FailedToEnsurePostState(
                        Box::new(ExecutionReport {
                            result: TxResult::Success,
                            gas_used: 42,
                            gas_refunded: 42,
                            logs: vec![],
                            output: Bytes::new(),
                        }),
                        //TODO: This is not a TransactionReport because it is REVM
                        error_reason,
                        HashMap::new(),
                    ));
                }
                // Execution result was successful and no exception was expected.
                None => {
                    let revm_account_updates =
                        backends::revm::REVM::get_state_transitions(revm_state);
                    let pos_state_root = post_state_root(&revm_account_updates, test).await;
                    let expected_post_state_root_hash =
                        test.post.vector_post_value(vector, *fork).hash;
                    if expected_post_state_root_hash != pos_state_root {
                        println!("Post-state root mismatch",);
                        return Err(EFTestRunnerError::FailedToEnsurePostState(
                            Box::new(ExecutionReport {
                                result: TxResult::Success,
                                gas_used: 42,
                                gas_refunded: 42,
                                logs: vec![],
                                output: Bytes::new(),
                            }),
                            //TODO: This is not a TransactionReport because it is REVM
                            "Post-state root mismatch".to_string(),
                            HashMap::new(),
                        ));
                    }
                }
            }
        }
        Err(err) => {
            match test.post.vector_post_value(vector, *fork).expect_exception {
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
                        ethrex_levm::errors::InternalError::Custom(format!(
                            "Unexpected exception: {err:?}",
                        ))
                        .into(), //TODO: Use another kind of error for this.
                    ));
                }
            }
        }
    };
    Ok(())
}
