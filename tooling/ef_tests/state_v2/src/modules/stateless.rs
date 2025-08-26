use bytes::Bytes;
use ethrex_blockchain::{Blockchain, BlockchainType, fork_choice::apply_fork_choice};
use ethrex_common::constants::DEFAULT_REQUESTS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, Receipt, compute_receipts_root, compute_transactions_root,
};
use ethrex_common::{Address, H256};
use ethrex_levm::{
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_vm::EvmEngine;
use std::str::FromStr;

use crate::modules::{
    error::RunnerError,
    result_check::check_test_case_results,
    runner::{get_tx_from_test_case, get_vm_env_for_test},
    types::Test,
    utils::load_initial_state,
};

pub async fn run_tests(tests: Vec<Test>) -> Result<(), RunnerError> {
    // Remove previous report if it exists.
    // This is for testing purposes
    let test = tests
        .iter()
        .find(|t| t.name.contains("fork_Prague") && !t.test_cases.is_empty())
        .expect("No test with name containing 'fork_Prague' and non-empty test_cases");

    // run_test(&test).await?;
    block_run(test).await?;

    Ok(())
}

pub async fn block_run(test: &Test) -> Result<(), RunnerError> {
    println!("Test name: {}", test.name);
    let test_case = &test.test_cases[0];
    let env = get_vm_env_for_test(test.env, test_case)?;
    let tx = get_tx_from_test_case(test_case).await?;
    let tracer = LevmCallTracer::disabled();

    // Note that this db is
    let (mut db, initial_block_hash, store, genesis) =
        load_initial_state(test, &test_case.fork).await;
    // Normal run cause we want to get the execution report.
    let mut vm =
        VM::new(env.clone(), &mut db, &tx, tracer, VMType::L1).map_err(RunnerError::VMError)?;
    let execution_result = vm.execute();

    let report = match execution_result {
        Ok(report) => report,
        Err(_) => {
            println!("Error in execution, we don't want to run with SP1.");
            return Ok(());
        }
    };

    let receipt = Receipt::new(
        tx.tx_type(),
        report.is_success(),
        report.gas_used,
        report.logs.clone(),
    );

    let transactions = vec![tx];
    let computed_tx_root = compute_transactions_root(&transactions);
    let body = BlockBody {
        transactions,
        ..Default::default()
    };

    let header = BlockHeader {
        hash: Default::default(), // I initialize it later with block.hash().
        parent_hash: initial_block_hash,
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: env.coinbase,
        state_root: test_case.post.hash,
        transactions_root: computed_tx_root,
        receipts_root: compute_receipts_root(&[receipt]),
        logs_bloom: Default::default(),
        difficulty: env.difficulty,
        number: 1, // I think this is correct
        gas_limit: env.block_gas_limit,
        gas_used: report.gas_used,
        timestamp: env.timestamp.try_into().unwrap(),
        extra_data: Bytes::new(),
        prev_randao: env.prev_randao.unwrap_or_default(),
        nonce: 0,
        base_fee_per_gas: Some(env.base_fee_per_gas.try_into().unwrap()),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used: Some(env.block_blob_gas_used.map(|v| v.as_u64()).unwrap_or(0)), //TODO: Blob gas used should only be post Cancun
        excess_blob_gas: env.block_excess_blob_gas.map(|v| v.as_u64()),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(*DEFAULT_REQUESTS_HASH),
    };
    header.hash();

    let block = Block::new(header, body);
    let hash = block.hash();

    let blockchain = Blockchain::new(EvmEngine::LEVM, store.clone(), BlockchainType::L1);

    blockchain
        .add_block(&block)
        .await
        .expect("Execution shouldn't fail :D");

    apply_fork_choice(&store, hash, hash, hash).await.unwrap();

    Ok(())
}
