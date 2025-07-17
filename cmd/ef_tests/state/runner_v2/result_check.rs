use std::collections::HashMap;

use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    types::{Account, AccountUpdate},
};
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    errors::{ExecutionReport, TxValidationError, VMError},
    vm::VM,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use ethrex_vm::backends;
use keccak_hash::{H256, keccak};

use crate::runner_v2::{
    error::RunnerError,
    types::{TestCase, TransactionExpectedException},
};

pub struct PostCheckResult {
    pub passed: bool,
    pub root_dif: Option<(H256, H256)>,
    pub accounts_diff: Option<Vec<AccountMismatch>>,
    pub logs_diff: Option<(H256, H256)>,
    pub exception_diff: Option<(Vec<TransactionExpectedException>, Option<VMError>)>,
}
impl Default for PostCheckResult {
    fn default() -> Self {
        Self {
            passed: true,
            root_dif: None,
            accounts_diff: None,
            logs_diff: None,
            exception_diff: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct AccountMismatch {
    pub address: Address,
    pub expected_balance: U256,
    pub actual_balance: U256,
    pub expected_nonce: u64,
    pub actual_nonce: u64,
    pub expected_code: Bytes,
    pub actual_code: Bytes,
    pub expected_storage: HashMap<U256, U256>,
    pub actual_storage: HashMap<H256, U256>,
}

/// Verify if the test has reached the expected results: if an exception was expected, check it was the corresponding
/// exception. If no exception was expected verify the result root.
pub async fn check_test_case_results(
    vm: &mut VM<'_>,
    initial_block_hash: H256,
    store: Store,
    test_case: &TestCase,
    execution_result: Result<ExecutionReport, VMError>,
) -> Result<PostCheckResult, RunnerError> {
    let mut checks_result = PostCheckResult::default();

    if test_case.expects_exception() {
        // Verify in case an exception was expected.
        check_exception(
            test_case.post.expected_exceptions.clone().unwrap(),
            execution_result,
            &mut checks_result,
        );
        Ok(checks_result)
    } else {
        // Verify hashed logs.
        check_logs(
            test_case,
            &execution_result.clone().unwrap(),
            &mut checks_result,
        );
        // Verify accounts' post state.
        check_accounts_state(vm.db, test_case, &mut checks_result);
        // Verify expected root hash.
        check_root(vm, initial_block_hash, store, test_case, &mut checks_result).await?;
        Ok(checks_result)
    }
}

pub async fn check_root(
    vm: &mut VM<'_>,
    initial_block_hash: H256,
    store: Store,
    test_case: &TestCase,
    check_result: &mut PostCheckResult,
) -> Result<(), RunnerError> {
    let account_updates = backends::levm::LEVM::get_state_transitions(vm.db)
        .map_err(|_| RunnerError::FailedToGetAccountsUpdates)?;
    let post_state_root = post_state_root(&account_updates, initial_block_hash, store).await;
    if post_state_root != test_case.post.hash {
        check_result.passed = false;
        check_result.root_dif = Some((test_case.post.hash, post_state_root));
    }
    Ok(())
}

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

pub fn check_exception(
    expected_exceptions: Vec<TransactionExpectedException>,
    execution_result: Result<ExecutionReport, VMError>,
    check_result: &mut PostCheckResult,
) {
    if execution_result.is_err() {
        let execution_err = execution_result.err().unwrap();
        if !exception_is_expected(expected_exceptions.clone(), execution_err.clone()) {
            check_result.exception_diff = Some((expected_exceptions, Some(execution_err)));
            check_result.passed = false;
        }
    } else {
        check_result.exception_diff = Some((expected_exceptions, None));
        check_result.passed = false;
    }
}

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

pub fn check_accounts_state(
    db: &GeneralizedDatabase,
    test_case: &TestCase,
    check_result: &mut PostCheckResult,
) {
    let mut accounts_diff = Vec::new();
    if test_case.post.state.is_some() {
        let expected_accounts_state = test_case.post.state.clone().unwrap();
        let current_accounts_state = db.current_accounts_state.clone();
        for (addr, state) in expected_accounts_state {
            let current_state = if current_accounts_state.contains_key(&addr) {
                current_accounts_state.get(&addr).unwrap()
            } else {
                // This should make checks comparison fail
                &Account::default()
            };
            let code_matches = current_state.code == state.code;
            let balance_matches = current_state.info.balance == state.balance;
            let nonce_matches = current_state.info.nonce == state.nonce;
            let mut storage_matches = true;
            for (storage_key, content) in &state.storage {
                let key = &H256::from(storage_key.to_big_endian());
                if current_state.storage.contains_key(key) {
                    if current_state.storage.get(key).unwrap() != content {
                        storage_matches = false;
                    }
                } else {
                    storage_matches = false;
                }
            }
            if !(code_matches && balance_matches && nonce_matches && storage_matches) {
                let account_mismatch = AccountMismatch {
                    address: addr,
                    expected_balance: state.balance,
                    actual_balance: current_state.info.balance,
                    expected_nonce: state.nonce,
                    actual_nonce: current_state.info.nonce,
                    expected_code: state.code,
                    actual_code: current_state.code.clone(),
                    expected_storage: state.storage,
                    actual_storage: current_state.storage.clone(),
                };
                accounts_diff.push(account_mismatch);
            }
        }
    }
    if !accounts_diff.is_empty() {
        check_result.accounts_diff = Some(accounts_diff);
        check_result.passed = false;
    }
}

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
