use std::collections::BTreeMap;

use ethrex_common::{
    Address, U256,
    types::{AccountUpdate, Fork, Genesis, code_hash},
};
use ethrex_levm::{
    account::LevmAccount,
    db::gen_db::GeneralizedDatabase,
    errors::{ExecutionReport, TxValidationError, VMError},
    vm::VM,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use ethrex_vm::backends;
use keccak_hash::{H256, keccak};

use crate::modules::{
    error::RunnerError,
    types::{AccountState, TestCase, TransactionExpectedException},
};

/// Keeps record of the post checks results for a test case, including if it passed
/// and if it did not, the differences from the expected post state.
pub struct PostCheckResult {
    pub fork: Fork,
    pub vector: (usize, usize, usize),
    pub passed: bool,
    pub root_diff: Option<(H256, H256)>,
    pub accounts_diff: Option<Vec<AccountMismatch>>,
    pub logs_diff: Option<(H256, H256)>,
    pub exception_diff: Option<(Vec<TransactionExpectedException>, Option<VMError>)>,
}
impl Default for PostCheckResult {
    fn default() -> Self {
        Self {
            fork: Fork::Prague,
            vector: (0, 0, 0),
            passed: true,
            root_diff: None,
            accounts_diff: None,
            logs_diff: None,
            exception_diff: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountMismatch {
    pub address: Address,
    pub balance_diff: Option<(U256, U256)>,
    pub nonce_diff: Option<(u64, u64)>,
    pub code_diff: Option<(H256, H256)>,
    pub storage_diff: Option<(BTreeMap<H256, U256>, BTreeMap<H256, U256>)>,
}

/// Verify if the test has reached the expected results: if an exception was expected, check it was the corresponding
/// exception. If no exception was expected verify the result root.
pub async fn check_test_case_results(
    vm: &mut VM<'_>,
    initial_block_hash: H256,
    store: Store,
    test_case: &TestCase,
    execution_result: Result<ExecutionReport, VMError>,
    genesis: Genesis,
) -> Result<PostCheckResult, RunnerError> {
    let mut checks_result = PostCheckResult {
        fork: test_case.fork,
        vector: test_case.vector,
        ..Default::default()
    };

    if let Some(expected_exceptions) = test_case.post.expected_exceptions.clone() {
        // Verify in case an exception was expected.
        check_exception(expected_exceptions, execution_result, &mut checks_result);
        Ok(checks_result)
    } else {
        // Verify expected root hash.
        check_root(vm, initial_block_hash, store, test_case, &mut checks_result).await?;
        // We only compare the other fields if there is a root mismatch, otherwise, test has passed.
        if !checks_result.passed {
            // Verify hashed logs.
            check_logs(
                test_case,
                &execution_result.clone().unwrap(),
                &mut checks_result,
            );

            // Verify accounts' post state.
            check_accounts_state(&mut vm.db.clone(), test_case, &mut checks_result, genesis);
        }

        Ok(checks_result)
    }
}

/// Verifies that the root of the state after executing the tests is the one expected
/// (the one that appears in the `.json` file).
pub async fn check_root(
    vm: &mut VM<'_>,
    initial_block_hash: H256,
    store: Store,
    test_case: &TestCase,
    check_result: &mut PostCheckResult,
) -> Result<(), RunnerError> {
    let account_updates = backends::levm::LEVM::get_state_transitions(&mut vm.db.clone())
        .map_err(|e| RunnerError::FailedToGetAccountsUpdates(e.to_string()))?;
    let post_state_root = post_state_root(&account_updates, initial_block_hash, store).await;
    if post_state_root != test_case.post.hash {
        check_result.passed = false;
        check_result.root_diff = Some((test_case.post.hash, post_state_root));
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
    check_result: &mut PostCheckResult,
) {
    if execution_result.is_err() {
        let execution_err = execution_result.err().unwrap();
        if !exception_matches_expected(expected_exceptions.clone(), execution_err.clone()) {
            check_result.exception_diff = Some((expected_exceptions, Some(execution_err)));
            check_result.passed = false;
        }
    } else {
        check_result.exception_diff = Some((expected_exceptions, None));
        check_result.passed = false;
    }
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

/// Verifies the hash of the output logs is the one expected.
pub fn check_logs(
    test_case: &TestCase,
    execution_report: &ExecutionReport,
    checks_result: &mut PostCheckResult,
) {
    let mut encoded_logs = Vec::new();
    execution_report.logs.encode(&mut encoded_logs);
    let hashed_logs = keccak(encoded_logs);
    if test_case.post.logs != hashed_logs {
        checks_result.passed = false;
        checks_result.logs_diff = Some((test_case.post.logs, hashed_logs));
    }
}

/// If the test case provides a `state` field in the post section, check account by account
/// its state is the one expected after the execution of the transaction.
pub fn check_accounts_state(
    db: &mut GeneralizedDatabase,
    test_case: &TestCase,
    check_result: &mut PostCheckResult,
    genesis: Genesis,
) {
    // In this case, the test in the .json file does not have post account details to verify.
    let Some(expected_accounts_state) = test_case.post.state.clone() else {
        return;
    };
    let mut accounts_diff: Vec<AccountMismatch> = Vec::new();

    // For every account in the test case expected post state, compare it to the actual state.
    for (addr, expected_account) in expected_accounts_state {
        let acc_genesis_state = genesis.alloc.get(&addr);
        // First we check if the address appears in the `current_accounts_state`, which stores accounts modified by the tx.
        let account: &mut LevmAccount =
            if let Some(account) = db.current_accounts_state.get_mut(&addr) {
                if account.storage.is_empty() && acc_genesis_state.is_some() {
                    account.storage = acc_genesis_state
                        .unwrap()
                        .storage
                        .iter()
                        .map(|(k, v)| (H256::from(k.to_big_endian()), *v))
                        .collect();
                }
                account
            } else {
                // Else, we take its info from the Genesis state, assuming it has not changed.
                if let Some(account) = acc_genesis_state {
                    &mut Into::<LevmAccount>::into(account.clone())
                } else {
                    // If we can't find it in any of the previous mappings, we provide a default account that will not pass
                    // the comparisons checks.
                    &mut LevmAccount::default()
                }
            };

        // We verify if the account matches the expected post state.
        let account_mismatch = verify_matching_accounts(addr, account, &expected_account);
        if let Some(mismatch) = account_mismatch {
            accounts_diff.push(mismatch);
        }
    }

    // If any of the accounts comparisons produced a mismatch, register it in the checks result.
    // If we got to this point, root did not match, therefore the test has already been registered
    // as failing, we do not need to set it again and just add the account diff.
    if !accounts_diff.is_empty() {
        check_result.accounts_diff = Some(accounts_diff);
    }
}

/// Compare every field of the expected account vs. the actual obtained account state.
fn verify_matching_accounts(
    addr: Address,
    actual_account: &LevmAccount,
    expected_account: &AccountState,
) -> Option<AccountMismatch> {
    let mut formatted_expected_storage = BTreeMap::new();
    for (key, value) in &expected_account.storage {
        let formatted_key = H256::from(key.to_big_endian());
        formatted_expected_storage.insert(formatted_key, *value);
    }
    let mut account_mismatch = AccountMismatch::default();
    let code_matches = actual_account.info.code_hash == keccak(&expected_account.code);
    let balance_matches = actual_account.info.balance == expected_account.balance;
    let nonce_matches = actual_account.info.nonce == expected_account.nonce;
    let storage_matches = formatted_expected_storage == actual_account.storage;

    if !code_matches {
        account_mismatch.code_diff = Some((
            code_hash(&expected_account.code),
            actual_account.info.code_hash,
        ));
    }
    if !balance_matches {
        account_mismatch.balance_diff =
            Some((expected_account.balance, actual_account.info.balance));
    }
    if !nonce_matches {
        account_mismatch.nonce_diff = Some((expected_account.nonce, actual_account.info.nonce));
    }
    if !storage_matches {
        account_mismatch.storage_diff =
            Some((formatted_expected_storage, actual_account.storage.clone()));
    }

    if account_mismatch != AccountMismatch::default() {
        account_mismatch.address = addr;
        Some(account_mismatch)
    } else {
        None
    }
}
