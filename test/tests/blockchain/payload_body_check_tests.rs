//! Blocks assembled from an engine payload skip the check of their body against
//! their header, because the header's transactions root, withdrawals root and
//! ommers were computed from that same body. These tests pin both halves of that:
//! every other import path still rejects a body that does not match its header,
//! and a block that went through the real payload round trip imports through the
//! engine entry point, with and without a witness.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    error::{ChainError, InvalidBlockError},
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, InvalidBlockBodyError,
    },
};
use ethrex_rpc::types::payload::ExecutionPayload;
use ethrex_storage::{EngineType, Store};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn test_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let genesis =
        serde_json::from_reader(BufReader::new(file)).expect("Failed to deserialize genesis file");
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}

/// A valid block on `parent`, built by the local payload builder.
fn build_block(store: &Store, parent: &BlockHeader) -> Block {
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
    let block = create_payload(&args, store, Bytes::new()).expect("payload creation");
    blockchain
        .build_payload(block)
        .expect("payload build")
        .payload
}

/// The block the engine handler would hand the executor for `block`: encoded as
/// an execution payload, then assembled back with `into_block`, which derives
/// the header's body commitments from the body. The handler rejects the payload
/// when the resulting hash differs from the claimed one; so does this helper.
fn through_payload(block: &Block) -> Block {
    let payload = ExecutionPayload::from_block(block.clone(), None);
    let claimed_hash = payload.block_hash;
    let assembled = payload
        .into_block(
            block.header.parent_beacon_block_root,
            block.header.requests_hash,
            block.header.block_access_list_hash,
        )
        .expect("payload decodes");
    assert_eq!(
        assembled.hash(),
        claimed_hash,
        "the engine handler would reject this payload"
    );
    assembled
}

fn assert_transactions_root_mismatch(result: Result<(), ChainError>) {
    match result {
        Err(ChainError::InvalidBlock(InvalidBlockError::InvalidBody(
            InvalidBlockBodyError::TransactionsRootNotMatch,
        ))) => {}
        other => panic!("expected a transactions root mismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn non_payload_imports_reject_a_body_that_does_not_match_its_header() {
    let store = test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let mut block = build_block(&store, &genesis);
    block.header.transactions_root = H256::repeat_byte(0x11);

    assert_transactions_root_mismatch(blockchain.add_block_pipeline(block.clone(), None));
    assert_transactions_root_mismatch(
        blockchain
            .add_block_pipeline_bounded(block.clone(), None, 128)
            .map(|_| ()),
    );
    assert_transactions_root_mismatch(blockchain.add_block_pipeline_bal(block, None).map(|_| ()));
}

#[tokio::test]
async fn payload_assembled_blocks_import_through_the_engine_entry_point() {
    let store = test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let block_1 = through_payload(&build_block(&store, &genesis));
    let witness = blockchain
        .add_block_pipeline_from_payload(block_1.clone(), None, None, false)
        .expect("block 1 imports");
    assert!(witness.is_none(), "no witness was requested");
    assert_eq!(
        store.get_block_header_by_hash(block_1.hash()).unwrap(),
        Some(block_1.header.clone()),
        "block 1 is stored"
    );

    let block_2 = through_payload(&build_block(&store, &block_1.header));
    let witness = blockchain
        .add_block_pipeline_from_payload(block_2.clone(), None, None, true)
        .expect("block 2 imports");
    assert!(witness.is_some(), "a witness was requested");
    assert!(
        store
            .get_block_header_by_hash(block_2.hash())
            .unwrap()
            .is_some(),
        "block 2 is stored"
    );
}

/// The engine entry point only skips the check for blocks whose header was built
/// from their body. Debug builds still run it, so passing any other block there
/// fails loudly instead of importing a block that does not match its header.
#[cfg(debug_assertions)]
#[tokio::test]
#[should_panic(expected = "does not match that header")]
async fn the_engine_entry_point_catches_misuse_in_debug_builds() {
    let store = test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let mut block = build_block(&store, &genesis);
    block.header.transactions_root = H256::repeat_byte(0x11);

    let _ = blockchain.add_block_pipeline_from_payload(block, None, None, false);
}
