use bytes::Bytes;
use ethrex_blockchain::{Blockchain, BlockchainType, fork_choice::apply_fork_choice};
use ethrex_common::constants::DEFAULT_REQUESTS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, Fork, Receipt, compute_receipts_root, compute_transactions_root,
};
use ethrex_common::{H256, U256};
use ethrex_levm::{
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_vm::EvmEngine;
use std::str::FromStr;

use crate::modules::types::TestCase;
use crate::modules::{
    error::RunnerError,
    runner::{get_tx_from_test_case, get_vm_env_for_test},
    types::Test,
    utils::load_initial_state,
};

pub async fn run_tests(tests: Vec<Test>) -> Result<(), RunnerError> {
    for test in &tests {
        for test_case in &test.test_cases {
            single_block_run(test, test_case).await?;
        }
    }

    Ok(())
}

pub async fn single_block_run(test: &Test, test_case: &TestCase) -> Result<(), RunnerError> {
    println!("Test name: {}", test.name);
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

    let fork = test_case.fork;
    let (excess_blob_gas, blob_gas_used, parent_beacon_block_root, requests_hash) = match fork {
        Fork::Prague | Fork::Cancun => {
            let excess_blob_gas = Some(
                test.env
                    .current_excess_blob_gas
                    .unwrap_or_default()
                    .as_u64(),
            );
            let blob_gas_used = Some(env.block_blob_gas_used.map(|v| v.as_u64()).unwrap_or(0));
            let parent_beacon_block_root = Some(H256::zero());
            let requests_hash = if fork == Fork::Prague {
                Some(*DEFAULT_REQUESTS_HASH)
            } else {
                None
            };
            (
                excess_blob_gas,
                blob_gas_used,
                parent_beacon_block_root,
                requests_hash,
            )
        }
        _ => (None, None, None, None),
    };

    let header = BlockHeader {
        hash: Default::default(), // I initialize it later with block.hash().
        parent_hash: initial_block_hash,
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: test.env.current_coinbase,
        state_root: test_case.post.hash,
        transactions_root: computed_tx_root,
        receipts_root: compute_receipts_root(&[receipt]),
        logs_bloom: Default::default(),
        difficulty: U256::zero(),
        number: 1, // I think this is correct
        gas_limit: test.env.current_gas_limit,
        gas_used: report.gas_used,
        timestamp: test.env.current_timestamp.as_u64(),
        extra_data: Bytes::new(),
        prev_randao: env.prev_randao.unwrap_or_default(),
        nonce: 0,
        base_fee_per_gas: test.env.current_base_fee.map(|f| f.as_u64()),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used, //TODO: I think for this I need to do a pre-execution to know blob gas used? Does it matter?
        excess_blob_gas,
        parent_beacon_block_root,
        requests_hash,
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
