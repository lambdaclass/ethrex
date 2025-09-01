use colored::Colorize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use ethrex_common::{
    U256,
    types::{EIP1559Transaction, EIP7702Transaction, Transaction, TxKind},
};
use ethrex_levm::{EVMConfig, Environment, tracing::LevmCallTracer, vm::VM, vm::VMType};

use crate::modules::{
    error::RunnerError,
    report::add_test_to_report,
    result_check::check_test_case_results,
    types::{Env, Test, TestCase},
    utils::{effective_gas_price, load_initial_state},
};

/// Runs all the tests that have been parsed.
pub async fn run_tests(tests: Vec<Test>) -> Result<(), RunnerError> {
    // Remove previous report if it exists.
    let successful_report_path = PathBuf::from("./success_report.txt");
    let _ = fs::remove_file(&successful_report_path);
    let _ = fs::remove_file("./failure_report.txt");

    let mut success_report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(successful_report_path)
        .unwrap();
    success_report
        .write_all("Successful tests: \n".as_bytes())
        .unwrap();
    let mut passing_tests = 0;
    let mut failing_tests = 0;
    let mut total_run = 0;

    //Test with the Fusaka tests that should pass. TODO: Once we've implemented all the Fusaka EIPs this should be removed
    //EIPs should be added as strings in the format 'eip-XXXX'
    let fusaka_eips_to_test: Vec<&str> = vec!["eip-7939", "eip-7883"];

    for test in tests {
        let test_eip = test._info.clone().reference_spec.unwrap_or_default();

        if test.path.to_str().unwrap().contains("osaka")
            && !fusaka_eips_to_test.iter().any(|eip| test_eip.contains(eip))
        {
            continue;
        }
        run_test(
            &test,
            &mut passing_tests,
            &mut failing_tests,
            &mut total_run,
        )
        .await?;
    }
    Ok(())
}

/// Runs each individual test case (combination of <fork, transaction, post-state>) of a specific test.
pub async fn run_test(
    test: &Test,
    passing_tests: &mut usize,
    failing_tests: &mut usize,
    total_run: &mut usize,
) -> Result<(), RunnerError> {
    let mut failing_test_cases = Vec::new();
    for test_case in &test.test_cases {
        // Setup VM for transaction.
        let (mut db, initial_block_hash, storage, genesis) = load_initial_state(test).await;
        let env = get_vm_env_for_test(test.env, test_case)?;
        let tx = get_tx_from_test_case(test_case)?;
        let tracer = LevmCallTracer::disabled();
        let mut vm =
            VM::new(env, &mut db, &tx, tracer, VMType::L1).map_err(RunnerError::VMError)?;

        // Execute transaction with VM.
        let execution_result = vm.execute();

        // Verify transaction execution results where the ones expected by the test case.
        let checks_result = check_test_case_results(
            &mut vm,
            initial_block_hash,
            storage,
            test_case,
            execution_result,
            genesis,
        )
        .await?;

        // If test case did not pass the checks, add it to failing test cases record (for future reporting)
        if !checks_result.passed {
            failing_test_cases.push(checks_result);
            *failing_tests += 1;
        } else {
            *passing_tests += 1;
        }
        *total_run += 1;

        print!(
            "\rTotal tests ran: {} - Total passed: {} - Total failed: {}",
            format!("{}", total_run).blue(),
            format!("{}", passing_tests).green(),
            format!("{}", failing_tests).red()
        );
    }
    add_test_to_report((test, failing_test_cases))?;

    Ok(())
}

/// Gets the enviroment needed to prepare the VM for a transaction.
pub fn get_vm_env_for_test(
    test_env: Env,
    test_case: &TestCase,
) -> Result<Environment, RunnerError> {
    let blob_schedule = EVMConfig::canonical_values(test_case.fork);
    let config = EVMConfig::new(test_case.fork, blob_schedule);
    let gas_price = effective_gas_price(&test_env, test_case)?;
    Ok(Environment {
        origin: test_case.sender,
        gas_limit: test_case.gas,
        config,
        block_number: test_env.current_number,
        coinbase: test_env.current_coinbase,
        timestamp: test_env.current_timestamp,
        prev_randao: test_env.current_random,
        difficulty: test_env.current_difficulty,
        chain_id: U256::from(1),
        base_fee_per_gas: test_env.current_base_fee.unwrap_or_default(),
        gas_price,
        block_excess_blob_gas: test_env.current_excess_blob_gas,
        block_blob_gas_used: None,
        tx_blob_hashes: test_case.blob_versioned_hashes.clone(),
        tx_max_priority_fee_per_gas: test_case.max_priority_fee_per_gas,
        tx_max_fee_per_gas: test_case.max_fee_per_gas,
        tx_max_fee_per_blob_gas: test_case.max_fee_per_blob_gas,
        tx_nonce: test_case.nonce,
        block_gas_limit: test_env.current_gas_limit,
        is_privileged: false,
    })
}

/// Constructs the transaction that will be executed in a specific test case.
pub fn get_tx_from_test_case(test_case: &TestCase) -> Result<Transaction, RunnerError> {
    let value = test_case.value;
    let data = test_case.data.clone();
    let access_list = test_case
        .access_list
        .iter()
        .map(|list_item| (list_item.address, list_item.storage_keys.clone()))
        .collect();

    // To simplify things, we represent all transactions using only two internal types.
    // Transactions of type 0 (legacy), 1 (EIP-2930), 2 (EIP-1559), and 3 (EIP-4844) are all
    // treated as EIP-1559-style transactions.
    // For type 3 transactions (EIP-4844), which include blobs, the difference is captured via
    // optional blob-related VM environment variables — these are set to Some() instead of None (check `get_vm_env_for_test()`).
    // Transactions of type 4 (EIP-7702) are represented using their actual type.
    // This approach avoids the need to handle five distinct transaction types separately.
    let tx = match &test_case.authorization_list {
        Some(list) => Transaction::EIP7702Transaction(EIP7702Transaction {
            to: match test_case.to {
                TxKind::Call(to) => to,
                TxKind::Create => return Err(RunnerError::EIP7702ShouldNotBeCreateType),
            },
            value,
            data,
            access_list,
            authorization_list: list
                .iter()
                .map(|auth_tuple| auth_tuple.clone().into_authorization_tuple())
                .collect(),
            ..Default::default()
        }),
        None => Transaction::EIP1559Transaction(EIP1559Transaction {
            to: test_case.to.clone(),
            value,
            data,
            access_list,
            ..Default::default()
        }),
    };
    Ok(tx)
}
