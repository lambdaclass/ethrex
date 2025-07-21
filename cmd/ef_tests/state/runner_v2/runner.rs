use std::fs;

use ethrex_common::{
    U256,
    types::{EIP1559Transaction, EIP7702Transaction, Transaction, TxKind},
};
use ethrex_levm::{EVMConfig, Environment, tracing::LevmCallTracer, vm::VM};

use crate::runner_v2::{
    error::RunnerError,
    report::create_report,
    result_check::check_test_case_results,
    types::{Env, Test, TestCase},
    utils::{effective_gas_price, load_initial_state},
};

pub async fn run_tests(tests: Vec<Test>) -> Result<(), RunnerError> {
    // Remove previous report if it exists.
    let _ = fs::remove_file("./runner_v2/success_report.txt");
    let _ = fs::remove_file("./runner_v2/failure_report.txt");

    for test in tests {
        println!("Executing test: {}", test.name);
        run_test(&test).await?;
    }
    Ok(())
}

pub async fn run_test(test: &Test) -> Result<(), RunnerError> {
    let mut failing_test_cases = Vec::new();
    for test_case in &test.test_cases {
        // Setup VM for transaction.
        let (mut db, initial_block_hash, storage, genesis) = load_initial_state(test).await;
        let env = get_vm_env_for_test(test.env, test_case)?;
        let tx = get_tx_from_test_case(test_case)?;
        let tracer = LevmCallTracer::disabled();
        let vm_type = ethrex_levm::vm::VMType::L1;
        let mut vm = VM::new(env.clone(), &mut db, &tx, tracer, vm_type);

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
            failing_test_cases.push((test_case.fork, checks_result));
        }
    }
    create_report((test, failing_test_cases))?;

    Ok(())
}

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

pub fn get_tx_from_test_case(test_case: &TestCase) -> Result<Transaction, RunnerError> {
    let value = test_case.value;
    let data = test_case.data.clone();
    let access_list = test_case
        .access_list
        .iter()
        .map(|list_item| (list_item.address, list_item.storage_keys.clone()))
        .collect();
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
