use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_blockchain::get_total_blob_gas;
use ethrex_common::constants::DEFAULT_REQUESTS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, Fork, Receipt, Transaction, compute_receipts_root,
    compute_transactions_root,
};
use ethrex_common::{H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use std::str::FromStr;
use std::sync::Arc;

thread_local! {
    /// Per-OS-thread merkleization pool, lazily built on first use. See the
    /// matching helper in `tooling/ef_tests/blockchain/test_runner.rs` for the
    /// reasoning; the merkle protocol requires exclusive ownership of its pool
    /// per concurrent caller, and keying by `thread_local!` provides that.
    static MERKLE_POOL: std::cell::OnceCell<Arc<rayon::ThreadPool>> =
        const { std::cell::OnceCell::new() };
}

fn merkle_pool() -> Arc<rayon::ThreadPool> {
    MERKLE_POOL.with(|cell| cell.get_or_init(Blockchain::build_merkle_pool).clone())
}

use crate::modules::types::TestCase;
use crate::modules::{
    error::RunnerError,
    runner::{get_tx_from_test_case, get_vm_env_for_test},
    types::Test,
    utils::load_initial_state,
};

pub async fn run_tests(tests: Vec<Test>) -> Result<(), RunnerError> {
    // Fusaka EIPs that block-mode supports; mirrors the allowlist in runner.rs.
    // TODO: drop once all Fusaka EIPs land.
    let fusaka_eips_to_test: Vec<&str> =
        vec!["eip-7594", "eip-7939", "eip-7918", "eip-7892", "eip-7883"];

    for test in &tests {
        // Apply the same gating runner.rs uses so we don't unconditionally run
        // every Osaka fixture in block mode. Fixtures without `_info` (e.g.
        // goevmlab-generated) bypass the filter — we can't read the EIP list,
        // so silently dropping them would be wrong.
        if test.path.to_str().unwrap().contains("osaka")
            && let Some(spec) = test
                ._info
                .as_ref()
                .and_then(|info| info.reference_spec.as_deref())
            && !fusaka_eips_to_test.iter().any(|eip| spec.contains(eip))
        {
            continue;
        }

        println!("Running test group: {}", test.name);
        for test_case in &test.test_cases {
            let res = run_test(test, test_case).await;
            if let Err(e) = res {
                println!("Error: {:?}", e);
            }
        }
    }

    Ok(())
}

pub async fn run_test(test: &Test, test_case: &TestCase) -> Result<(), RunnerError> {
    // 1. We need to do a pre-execution with LEVM because we need to know gas used and generate receipts for the block header.
    let env = get_vm_env_for_test(test.env, test_case)?;
    let tx = get_tx_from_test_case(test_case).await?;
    let tracer = LevmCallTracer::disabled();

    let (mut db, initial_block_hash, store, _genesis) =
        load_initial_state(test, &test_case.fork, false).await;
    let mut vm = VM::new(
        env.clone(),
        &mut db,
        &tx,
        tracer,
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .map_err(RunnerError::VMError)?;
    let execution_result = vm.execute();

    let (receipts, gas_used) = match execution_result {
        Ok(report) => {
            let receipt = Receipt::new(
                tx.tx_type(),
                report.is_success(),
                report.gas_used,
                report.logs.clone(),
            );
            (vec![receipt], report.gas_used)
        }
        Err(e) => {
            if test_case.post.expected_exceptions.is_some() {
                (vec![], 0)
            } else {
                return Err(RunnerError::Custom(format!("Internal error {e}")));
            }
        }
    };

    // 2. Set up Block Body and Block Header

    let transactions = vec![tx.clone()];
    let computed_tx_root = compute_transactions_root(&transactions, &ethrex_crypto::NativeCrypto);
    let body = BlockBody {
        transactions,
        ..Default::default()
    };

    let fork = test_case.fork;
    // These variables are Some or None depending on the fork.
    // So they could be specified in the test but if the fork is e.g. Paris we should set them to None despite that.
    // Otherwise it will fail block header validations
    let (excess_blob_gas, blob_gas_used, parent_beacon_block_root, requests_hash) = match fork {
        Fork::Cancun | Fork::Prague | Fork::Osaka => {
            let blob_gas_used = match tx {
                Transaction::EIP4844Transaction(blob_tx) => {
                    Some(get_total_blob_gas(&blob_tx) as u64)
                }
                _ => Some(0),
            };

            let excess_blob_gas = Some(
                test.env
                    .current_excess_blob_gas
                    .unwrap_or_default()
                    .try_into()
                    .unwrap(),
            );
            let parent_beacon_block_root = Some(H256::zero());
            // Prague added requests; Osaka inherits the same mechanism.
            let requests_hash = match fork {
                Fork::Prague | Fork::Osaka => Some(*DEFAULT_REQUESTS_HASH),
                _ => None,
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
        hash: Default::default(), // It is initialized later with block.hash().
        parent_hash: initial_block_hash,
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: test.env.current_coinbase,
        state_root: test_case.post.hash,
        transactions_root: computed_tx_root,
        receipts_root: compute_receipts_root(&receipts, &ethrex_crypto::NativeCrypto),
        logs_bloom: Default::default(),
        difficulty: U256::zero(),
        number: 1, // In Ethereum state tests, the block being constructed is always the first block after genesis, which has block number 1.
        gas_limit: test.env.current_gas_limit,
        gas_used,
        timestamp: test.env.current_timestamp.try_into().unwrap(),
        extra_data: Bytes::new(),
        prev_randao: test.env.current_random.unwrap_or_default(),
        nonce: 0,
        base_fee_per_gas: test.env.current_base_fee.map(|f| f.try_into().unwrap()),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used,
        excess_blob_gas,
        parent_beacon_block_root,
        requests_hash,
        block_access_list_hash: None,
        slot_number: None,
        burned_fees: None,
    };
    let block = Block::new(header, body);

    // 3. Create Blockchain and add block.

    let blockchain = Blockchain::default_with_store_and_pool(store, merkle_pool());

    let result = blockchain.add_block_pipeline(block, None);

    if result.is_err() && test_case.post.expected_exceptions.is_none() {
        return Err(RunnerError::Custom(
            "Execution failed but test didn't expect any error.".to_string(),
        ));
    }
    if test_case.post.expected_exceptions.is_some() && result.is_ok() {
        return Err(RunnerError::Custom(
            "Test expected an error but execution didn't fail.".to_string(),
        ));
    }

    Ok(())
}
