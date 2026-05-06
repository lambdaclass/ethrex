use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};

#[tokio::test]
async fn generated_witness_has_ancestor_headers_in_ascending_order() {
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let block_1 = new_block(&store, &genesis_header).await;
    blockchain.add_block(block_1.clone()).unwrap();
    let block_2 = new_block(&store, &block_1.header).await;
    blockchain.add_block(block_2.clone()).unwrap();
    let block_3 = new_block(&store, &block_2.header).await;
    blockchain.add_block(block_3.clone()).unwrap();

    let witness = blockchain
        .generate_witness_for_blocks(&[block_2.clone(), block_3.clone()])
        .await
        .unwrap();

    let numbers: Vec<u64> = witness
        .block_headers_bytes
        .iter()
        .map(|b| BlockHeader::decode(b).unwrap().number)
        .collect();

    assert!(
        numbers.windows(2).all(|w| w[0] < w[1]),
        "ancestor headers must be ascending, got {numbers:?}"
    );
}

async fn new_block(store: &Store, parent: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::random(),
        random: H256::random(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::random()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let blockchain = Blockchain::default_with_store(store.clone());
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(block).unwrap().payload
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn test_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis = serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}
