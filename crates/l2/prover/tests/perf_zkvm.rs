#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
use ethrex_blockchain::Blockchain;
use ethrex_common::types::Block;
use ethrex_prover_lib::execute;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{StoreWrapper, ToExecDB};
use std::path::Path;
use tracing::info;
use zkvm_interface::io::ProgramInput;

#[tokio::test]
async fn test_performance_zkvm() {
    tracing_subscriber::fmt::init();

    let (input, block_to_prove) = setup().await;

    let start = std::time::Instant::now();

    // this is only executing because these tests run as a CI job and should be fast
    // TODO: create a test for actual proving
    execute(input).unwrap();

    let duration = start.elapsed().as_secs();
    info!(
        "Number of transactions in the proven block: {}",
        block_to_prove.body.transactions.len()
    );
    info!(
        "Execution took {secs}s or {mins}m",
        secs = duration,
        mins = duration / 60
    );
}

async fn setup() -> (ProgramInput, Block) {
    let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test_data"));

    let genesis_file_path = path.join("genesis-l2-ci.json");
    // l2-loadtest.rlp has blocks with many txs.
    let chain_file_path = path.join("l2-loadtest.rlp");

    let store = Store::new("memory", EngineType::InMemory).expect("Failed to create Store");

    let genesis =
        ethrex_l2::utils::test_data_io::read_genesis_file(genesis_file_path.to_str().unwrap());
    store.add_initial_state(genesis.clone()).await.unwrap();

    let blocks = ethrex_l2::utils::test_data_io::read_chain_file(chain_file_path.to_str().unwrap());
    info!("Number of blocks to insert: {}", blocks.len());

    let blockchain = Blockchain::default_with_store(store.clone());
    for block in &blocks {
        info!(
            "txs {} in block{}",
            block.body.transactions.len(),
            block.header.number
        );
        blockchain.add_block(block).await.unwrap();
    }
    let block_to_prove = blocks.get(3).unwrap();

    let parent_block_header = store
        .get_block_header_by_hash(block_to_prove.header.parent_hash)
        .unwrap()
        .unwrap();

    let store = StoreWrapper {
        store: store.clone(),
        block_hash: block_to_prove.header.parent_hash,
    };
    let db = store.to_exec_db(block_to_prove).unwrap();

    let input = ProgramInput {
        block: block_to_prove.clone(),
        parent_block_header,
        db,
    };
    (input, block_to_prove.clone())
}
