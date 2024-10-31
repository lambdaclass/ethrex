use std::path::PathBuf;
use tracing::info;

use ethereum_rust_blockchain::add_block;
use ethereum_rust_l2::proposer::prover_server::ProverInputData;
use ethereum_rust_prover_lib::prover::Prover;
use ethereum_rust_storage::{EngineType, Store};
use ethereum_rust_vm::execution_db::ExecutionDB;

#[tokio::test]
async fn test_performance_zkvm() {
    tracing_subscriber::fmt::init();

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Go back 3 levels (Go to the root of the project)
    for _ in 0..3 {
        path.pop();
    }
    path.push("test_data");

    // Another use is genesis-execution-api.json in conjunction with chain.rlp(20 blocks not too loaded).
    let genesis_file_path = path.join("genesis-l2.json");
    // l2-loadtest.rlp has blocks with many txs.
    let chain_file_path = path.join("l2-loadtest.rlp");

    let store = Store::new("memory", EngineType::InMemory).expect("Failed to create Store");

    let genesis = ethereum_rust_l2::utils::test_data_io::read_genesis_file(
        genesis_file_path.to_str().unwrap(),
    );
    store.add_initial_state(genesis.clone()).unwrap();

    let blocks =
        ethereum_rust_l2::utils::test_data_io::read_chain_file(chain_file_path.to_str().unwrap());
    info!("Number of blocks to insert: {}", blocks.len());

    for block in &blocks {
        add_block(block, &store).unwrap();
    }
    let block_to_prove = blocks.last().unwrap();

    let db = ExecutionDB::from_exec(block_to_prove, &store).unwrap();

    let parent_header = store
        .get_block_header_by_hash(block_to_prove.header.parent_hash)
        .unwrap()
        .unwrap();

    let input = ProverInputData {
        db,
        block: block_to_prove.clone(),
        parent_header,
    };

    let mut prover = Prover::new();
    prover.set_input(input);

    let start = std::time::Instant::now();

    let receipt = prover.prove().unwrap();

    let duration = start.elapsed();
    info!(
        "Number of EIP1559 transactions in the proven block: {}",
        block_to_prove.body.transactions.len()
    );
    info!("[SECONDS] Proving Took: {:?}", duration);
    info!("[MINUTES] Proving Took: {}[m]", duration.as_secs() / 60);

    prover.verify(&receipt).unwrap();

    let output = Prover::get_commitment(&receipt).unwrap();

    let execution_cumulative_gas_used = output.block_receipts.last().unwrap().cumulative_gas_used;
    info!("Cumulative Gas Used {execution_cumulative_gas_used}");

    let gas_per_second = execution_cumulative_gas_used as f64 / duration.as_secs_f64();

    info!("Gas per Second: {}", gas_per_second);
}
