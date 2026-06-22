use ethrex_common::{
    H256, U256,
    types::{ForkId, Genesis},
};
use ethrex_p2p::backend::validate_status;
use ethrex_p2p::rlpx::eth::eth68::status::StatusMessage68;
use ethrex_p2p::rlpx::p2p::Capability;
use ethrex_storage::{EngineType, Store};
use std::path::PathBuf;
use std::{fs::File, io::BufReader};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[tokio::test]
async fn test_validate_status() {
    let mut storage =
        Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    storage
        .add_initial_state(genesis.clone())
        .await
        .expect("Failed to add genesis block to DB");
    let config = genesis.config;
    let total_difficulty = U256::from(config.terminal_total_difficulty.unwrap_or_default());
    let genesis_header = genesis.get_block().header;
    let genesis_hash = genesis_header.hash();
    let fork_id = ForkId::new(config, genesis_header, 2707305664, 123);

    let eth = Capability::eth(68);
    let message = StatusMessage68 {
        eth_version: eth.version,
        network_id: 3503995874084926,
        total_difficulty,
        block_hash: H256::random(),
        genesis: genesis_hash,
        fork_id,
    };
    let result = validate_status(message, &storage, &eth).await;
    assert!(result.is_ok());
}
