//! `engine_newPayload` for a block that is already on our canonical chain.
//!
//! When such a block's own state has been evicted but its parent's state is reachable,
//! `try_execute_payload` re-executes it to rebuild the evicted state. That block was
//! accepted when it first joined the canonical chain, so a failed re-execution says the
//! state we rebuilt for it is wrong, not that the block is invalid. Recording it as a bad
//! block would make the node reject its own canonical chain, and everything built on it,
//! until someone intervenes.
//!
//! The block used here is made to fail re-execution on purpose (its header claims a
//! state root execution cannot produce) and is installed as the canonical head without
//! being executed, which is the state a node is in when it re-executes an old block.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis},
};
use ethrex_rpc::{
    engine::payload::NewPayloadV5Request, rpc::RpcHandler,
    test_utils::default_context_with_storage, types::payload::ExecutionPayload,
};
use ethrex_storage::{EngineType, Store};
use serde_json::{Value, json};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn setup_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

fn build_on(store: &Store, parent: &BlockHeader, slot: u64) -> PayloadBuildResult {
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(slot),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(payload).unwrap()
}

fn new_payload_v5(block: Block, built: &PayloadBuildResult) -> NewPayloadV5Request {
    let payload = ExecutionPayload::from_block(block, built.block_access_list.clone());
    // The engine API carries only requests with data: a bare type byte is dropped.
    let requests: Vec<Value> = built
        .requests
        .iter()
        .filter(|r| r.0.len() > 1)
        .map(|r| serde_json::to_value(r).expect("request to json"))
        .collect();
    let params = Some(vec![
        serde_json::to_value(payload).expect("payload to json"),
        json!([]),
        json!(H256::zero()),
        Value::Array(requests),
    ]);
    NewPayloadV5Request::parse(&params).expect("well-formed newPayloadV5 params")
}

/// Block 1 executed and canonical, then a block 2 whose header cannot be reproduced by
/// execution (wrong state root), stored without executing it. When `canonical` is set,
/// block 2 is installed as the canonical head, as it would be on a node that accepted it
/// earlier and has since lost its state.
async fn chain_with_unreproducible_block_2(
    store: &Store,
    canonical: bool,
) -> (Block, PayloadBuildResult) {
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    let built_1 = build_on(store, &genesis_header, 1);
    let block_1 = built_1.payload.clone();
    let importer = Blockchain::new(store.clone(), BlockchainOptions::default());
    importer
        .add_block_pipeline_bal(
            block_1.clone(),
            built_1.block_access_list.clone().map(std::sync::Arc::new),
        )
        .expect("block 1 imports");
    store
        .forkchoice_update(vec![(1, block_1.hash())], 1, block_1.hash(), None, None)
        .await
        .expect("block 1 becomes the head");

    let built_2 = build_on(store, &block_1.header, 2);
    let mut block_2 = built_2.payload.clone();
    block_2.header.state_root = H256::repeat_byte(0xab);
    block_2.header.hash = Default::default();

    store
        .add_block(block_2.clone())
        .await
        .expect("store block 2");
    if canonical {
        store
            .forkchoice_update(vec![(2, block_2.hash())], 2, block_2.hash(), None, None)
            .await
            .expect("block 2 becomes the head");
    }
    (block_2, built_2)
}

#[tokio::test]
async fn failed_reexecution_of_a_canonical_block_does_not_mark_it_bad() {
    let store = setup_store().await;
    let (block_2, built_2) = chain_with_unreproducible_block_2(&store, true).await;
    let ctx = default_context_with_storage(store.clone()).await;

    let response = new_payload_v5(block_2.clone(), &built_2)
        .handle(ctx.clone())
        .await
        .expect("newPayload returns a payload status");

    assert_eq!(
        response["status"], "VALID",
        "a block already on our canonical chain must not be reported INVALID because \
         re-executing it failed, got {response:?}"
    );
    let bad: Vec<H256> = store
        .get_bad_blocks()
        .await
        .unwrap()
        .iter()
        .map(Block::hash)
        .collect();
    assert!(
        !bad.contains(&block_2.hash()),
        "it must not be recorded as a bad block"
    );
    assert_eq!(
        store
            .get_latest_valid_ancestor(block_2.hash())
            .await
            .unwrap(),
        None,
        "it must not be recorded as invalid, or its descendants would be rejected too"
    );
}

/// Control: the same block when it is NOT on our canonical chain is still rejected and
/// recorded, so the guard only covers blocks we had already accepted.
#[tokio::test]
async fn failed_execution_of_a_non_canonical_block_still_marks_it_bad() {
    let store = setup_store().await;
    let (block_2, built_2) = chain_with_unreproducible_block_2(&store, false).await;
    let ctx = default_context_with_storage(store.clone()).await;

    let response = new_payload_v5(block_2.clone(), &built_2)
        .handle(ctx.clone())
        .await
        .expect("newPayload returns a payload status");

    assert_eq!(response["status"], "INVALID", "got {response:?}");
    let bad: Vec<H256> = store
        .get_bad_blocks()
        .await
        .unwrap()
        .iter()
        .map(Block::hash)
        .collect();
    assert!(
        bad.contains(&block_2.hash()),
        "it must be recorded as a bad block"
    );
}
