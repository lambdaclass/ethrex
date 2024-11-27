use crate::{
    report::{AccountUpdatesReport, EFTestReport, TestReRunReport, TestVector},
    runner::{levm_runner, EFTestRunnerError, InternalError},
    types::EFTest,
    utils::load_initial_state,
};
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
        BlobExcessGasAndPrice, BlockEnv as RevmBlockEnv, EVMError as REVMError,
        ExecutionResult as RevmExecutionResult, TxEnv as RevmTxEnv, TxKind as RevmTxKind,
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
        blob_excess_gas_and_price: Some(BlobExcessGasAndPrice::new(
            test.env
                .current_excess_blob_gas
                .unwrap_or_default()
                .as_u64(),
        )),
    };
    let tx = &test
        .transactions
        .get(vector)
        .ok_or(EFTestRunnerError::VMInitializationFailed(format!(
            "Vector {vector:?} not found in test {}",
            test.name
        )))?;
    let tx_env = RevmTxEnv {
        caller: tx.sender.0.into(),
        gas_limit: tx.gas_limit.as_u64(),
        gas_price: RevmU256::from_limbs(tx.gas_price.unwrap_or_default().0),
        transact_to: match tx.to {
            TxKind::Call(to) => RevmTxKind::Call(to.0.into()),
            TxKind::Create => RevmTxKind::Create,
        },
        value: RevmU256::from_limbs(tx.value.0),
        data: tx.data.to_vec().into(),
        nonce: Some(tx.nonce.as_u64()),
        chain_id: None,
        access_list: Vec::default(),
        gas_priority_fee: None,
        blob_hashes: Vec::default(),
        max_fee_per_blob_gas: None,
        authorization_list: None,
    };
    let evm_builder = Revm::builder()
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain_id)
        .with_spec_id(SpecId::CANCUN)
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
            let levm_account_updates = levm_runner::get_state_transitions(levm_execution_report);
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
) -> AccountUpdatesReport {
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

    AccountUpdatesReport {
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
