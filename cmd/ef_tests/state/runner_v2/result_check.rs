use std::{fs::OpenOptions, io::Write, path::PathBuf};

use ethrex_common::types::AccountUpdate;
use ethrex_levm::{
    errors::{ExecutionReport, TxValidationError, VMError},
    vm::VM,
};
use ethrex_storage::Store;
use ethrex_vm::backends;
use keccak_hash::H256;

use crate::runner_v2::{
    error::RunnerError,
    types::{Test, TestCase, TransactionExpectedException},
};

/// Creates a basic `.txt` report that informs the result of executing each test (if it passed or failed).
pub fn create_report(
    res: Result<(), RunnerError>,
    test_case: &TestCase,
    test: &Test,
) -> Result<(), RunnerError> {
    let report_path = PathBuf::from("./runner_v2/runner_report.txt");
    let mut report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(report_path)
        .unwrap();
    let content = if res.is_ok() {
        format!(
            "Test checks succeded for test: {:?}, with fork {:?},  in path: {:?}.\n",
            test.name, test_case.fork, test.path
        )
    } else {
        format!(
            "Test checks failed for test: {:?}, with fork: {:?},  in path: {:?}.\n",
            test.name, test_case.fork, test.path,
        )
    };
    let _ = report.write_all(content.as_bytes());

    Ok(())
}

/// Verify if the test has given the expected results: if an exception was expected, check it was the corresponding
/// exception. If no exception was expected verify the result root.
pub async fn check_test_case_results(
    vm: &mut VM<'_>,
    initial_block_hash: H256,
    store: Store,
    test_case: &TestCase,
    execution_result: Result<ExecutionReport, VMError>,
) -> Result<(), RunnerError> {
    if let Some(expected_exceptions) = test_case.post.expected_exception.clone() {
        check_exception(expected_exceptions, execution_result)?;
    } else {
        // Verify expected root hash.
        check_root(vm, initial_block_hash, store, test_case).await?;
    }
    Ok(())
}

/// Verifies that the root of the state after executing the tests is the one expected
/// (the one that appears in the `.json` file).
pub async fn check_root(
    vm: &mut VM<'_>,
    initial_block_hash: H256,
    store: Store,
    test_case: &TestCase,
) -> Result<(), RunnerError> {
    let account_updates = backends::levm::LEVM::get_state_transitions(vm.db)
        .map_err(|_| RunnerError::FailedToGetAccountsUpdates)?;
    let post_state_root = post_state_root(&account_updates, initial_block_hash, store).await;
    if post_state_root != test_case.post.hash {
        return Err(RunnerError::RootMismatch);
    }
    Ok(())
}

/// Calculates the post state root applying the changes (the account updates) that are a
/// result of running the transaction to the storage.
pub async fn post_state_root(
    account_updates: &[AccountUpdate],
    initial_block_hash: H256,
    store: Store,
) -> H256 {
    let ret_account_updates_batch = store
        .apply_account_updates_batch(initial_block_hash, account_updates)
        .await
        .unwrap()
        .unwrap();
    ret_account_updates_batch.state_trie_hash
}

/// Used when the test case expected an exception. Verifies first if it, indeed, failed
/// and if it did if it failed with the corresponding error.
pub fn check_exception(
    expected_exceptions: Vec<TransactionExpectedException>,
    execution_result: Result<ExecutionReport, VMError>,
) -> Result<(), RunnerError> {
    if execution_result.is_ok() {
        return Err(RunnerError::TxSucceededAndExceptionWasExpected);
    } else if !exception_matches_expected(expected_exceptions, execution_result.err().unwrap()) {
        return Err(RunnerError::DifferentExceptionWasExpected);
    }
    Ok(())
}

/// Verifies whether a transaction execution error is contained in a vector of
/// expected exceptions.
fn exception_matches_expected(
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
                VMError::TxValidation(TxValidationError::PriorityGreaterThanMaxFeePerGas {
                    priority_fee: _,
                    max_fee_per_gas: _
                })
            ) | (
                TransactionExpectedException::GasLimitPriceProductOverflow,
                VMError::TxValidation(TxValidationError::GasLimitPriceProductOverflow)
            ) | (
                TransactionExpectedException::SenderNotEoa,
                VMError::TxValidation(TxValidationError::SenderNotEOA(_))
            ) | (
                TransactionExpectedException::InsufficientMaxFeePerGas,
                VMError::TxValidation(TxValidationError::InsufficientMaxFeePerGas)
            ) | (
                TransactionExpectedException::NonceIsMax,
                VMError::TxValidation(TxValidationError::NonceIsMax)
            ) | (
                TransactionExpectedException::GasAllowanceExceeded,
                VMError::TxValidation(TxValidationError::GasAllowanceExceeded {
                    block_gas_limit: _,
                    tx_gas_limit: _
                })
            ) | (
                TransactionExpectedException::Type3TxPreFork,
                VMError::TxValidation(TxValidationError::Type3TxPreFork)
            ) | (
                TransactionExpectedException::Type3TxBlobCountExceeded,
                VMError::TxValidation(TxValidationError::Type3TxBlobCountExceeded {
                    max_blob_count: _,
                    actual_blob_count: _
                })
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
                VMError::TxValidation(TxValidationError::InsufficientMaxFeePerBlobGas {
                    base_fee_per_blob_gas: _,
                    tx_max_fee_per_blob_gas: _
                })
            ) | (
                TransactionExpectedException::InitcodeSizeExceeded,
                VMError::TxValidation(TxValidationError::InitcodeSizeExceeded {
                    max_size: _,
                    actual_size: _
                })
            ) | (
                TransactionExpectedException::Type4TxContractCreation,
                VMError::TxValidation(TxValidationError::Type4TxContractCreation)
            ) | (
                TransactionExpectedException::Other,
                VMError::TxValidation(_) //TODO: Decide whether to support more specific errors, I think this is enough.
            )
        )
    })
}
