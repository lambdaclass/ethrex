//! `engine_newPayload` re-sent while the first call for the same block is still running.
//!
//! Consensus clients re-send a payload when the first call is slow. Whichever request
//! ends up executing the block, every copy must get the block's real answer, and a
//! witness only for the requests that asked for one.

use ethrex_rpc::test_utils::raw_params;
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
    engine::payload::{NewPayloadV5Request, NewPayloadWithWitnessV5Request},
    rpc::RpcHandler,
    test_utils::default_context_with_storage,
    types::payload::ExecutionPayload,
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

fn build_on(store: &Store, parent: &BlockHeader) -> PayloadBuildResult {
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
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
    NewPayloadV5Request::parse(&raw_params(&params)).expect("well-formed newPayloadV5 params")
}

fn latest_valid_hash(response: &Value) -> H256 {
    serde_json::from_value(response["latestValidHash"].clone()).expect("a latestValidHash")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_copy_of_a_valid_payload_is_answered_valid() {
    let store = setup_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let built = build_on(&store, &genesis);
    let block = built.payload.clone();
    let ctx = default_context_with_storage(store.clone()).await;
    let request = new_payload_v5(block.clone(), &built);

    let (first, second, third) = tokio::join!(
        request.handle(ctx.clone()),
        request.handle(ctx.clone()),
        request.handle(ctx.clone()),
    );

    for response in [first, second, third] {
        let response = response.expect("newPayload returns a payload status");
        assert_eq!(response["status"], "VALID", "got {response:?}");
        assert_eq!(latest_valid_hash(&response), block.hash());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_copy_of_an_invalid_payload_is_answered_invalid() {
    let store = setup_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let built = build_on(&store, &genesis);
    // A state root execution cannot produce.
    let mut block = built.payload.clone();
    block.header.state_root = H256::repeat_byte(0xab);
    block.header.hash = Default::default();
    let ctx = default_context_with_storage(store.clone()).await;
    let request = new_payload_v5(block.clone(), &built);

    let (first, second, third) = tokio::join!(
        request.handle(ctx.clone()),
        request.handle(ctx.clone()),
        request.handle(ctx.clone()),
    );

    for response in [first, second, third] {
        let response = response.expect("newPayload returns a payload status");
        assert_eq!(response["status"], "INVALID", "got {response:?}");
        assert_eq!(latest_valid_hash(&response), genesis.hash());
    }
    assert_eq!(
        store.get_latest_valid_ancestor(block.hash()).await.unwrap(),
        Some(genesis.hash()),
        "the block must be recorded as invalid so later copies are rejected without executing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_plain_and_witness_requests_each_get_what_they_asked_for() {
    let store = setup_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let built = build_on(&store, &genesis);
    let block = built.payload.clone();
    let ctx = default_context_with_storage(store.clone()).await;
    let plain = new_payload_v5(block.clone(), &built);
    let with_witness = NewPayloadWithWitnessV5Request(new_payload_v5(block.clone(), &built));

    let (plain, with_witness) =
        tokio::join!(plain.handle(ctx.clone()), with_witness.handle(ctx.clone()));

    let plain = plain.expect("newPayload returns a payload status");
    assert_eq!(plain["status"], "VALID", "got {plain:?}");
    assert!(plain.get("witness").is_none(), "got {plain:?}");

    let with_witness = with_witness.expect("newPayloadWithWitness returns a payload status");
    assert_eq!(with_witness["status"], "VALID", "got {with_witness:?}");
    assert!(
        with_witness.get("witness").is_some_and(Value::is_string),
        "got {with_witness:?}"
    );
}
