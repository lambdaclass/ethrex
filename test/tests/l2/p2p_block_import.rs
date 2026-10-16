//! Block sync over the L2 `based` capability across reconnects.
//!
//! A follower that already has blocks must keep importing new ones on a fresh connection, which
//! is what it gets whenever the sequencer or the follower restarts, and the sequencer should only
//! re-send what the follower is missing.

use std::{
    collections::BTreeMap,
    fs::File,
    io::BufReader,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256, H512,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, batch::Batch,
        fee_config::FeeConfig,
    },
};
use ethrex_p2p::{
    rlpx::l2::l2_connection::{
        L2ConnectedState, QueuedBlock, import_queued_blocks, peer_head_on_local_chain,
    },
    types::Node,
};
use ethrex_storage::{EngineType, Store};
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use secp256k1::SecretKey;
use tokio::time::Instant;

/// The follower has blocks 1..=5 and a sealed batch that only covers 1..=2, as when the sequencer
/// restarts between commits. On the new connection it must import 6 and 7 anyway.
#[tokio::test]
async fn new_connection_imports_blocks_past_the_local_head() {
    let sequencer_chain = build_chain(7).await;
    let (store, blockchain) = follower_with(&sequencer_chain[..5]).await;
    let mut l2_state = new_connection_state().await;
    l2_state
        .store_rollup
        .seal_batch(Batch {
            number: 1,
            first_block: 1,
            last_block: 2,
            ..Default::default()
        })
        .await
        .unwrap();

    // Block 7 arrives first: it has to wait for 6.
    queue(&mut l2_state, &sequencer_chain[6]);
    import_queued_blocks(&store, &blockchain, &mut l2_state, &peer())
        .await
        .unwrap();
    assert_eq!(store.get_latest_block_number().unwrap(), 5);
    assert!(l2_state.blocks_on_queue.contains_key(&7));

    queue(&mut l2_state, &sequencer_chain[5]);
    import_queued_blocks(&store, &blockchain, &mut l2_state, &peer())
        .await
        .unwrap();
    assert_eq!(store.get_latest_block_number().unwrap(), 7);
    assert_eq!(
        store.get_block_header(7).unwrap().unwrap().hash(),
        sequencer_chain[6].hash()
    );
    assert!(l2_state.blocks_on_queue.is_empty());
}

/// Blocks at or below the head (e.g. imported through another connection after they were queued
/// here) are dropped instead of blocking the queue.
#[tokio::test]
async fn blocks_already_imported_are_skipped() {
    let sequencer_chain = build_chain(7).await;
    let (store, blockchain) = follower_with(&sequencer_chain[..5]).await;
    let mut l2_state = new_connection_state().await;

    queue(&mut l2_state, &sequencer_chain[2]);
    queue(&mut l2_state, &sequencer_chain[5]);
    queue(&mut l2_state, &sequencer_chain[6]);
    // Another connection imports block 6 before this one gets to it.
    import_block(&blockchain, &store, sequencer_chain[5].clone()).await;

    import_queued_blocks(&store, &blockchain, &mut l2_state, &peer())
        .await
        .unwrap();
    assert_eq!(store.get_latest_block_number().unwrap(), 7);
    assert!(l2_state.blocks_on_queue.is_empty());
}

/// A reconnecting follower that is behind on our chain gets blocks from its head on.
#[tokio::test]
async fn broadcast_starts_at_a_peer_head_on_our_chain() {
    let chain = build_chain(5).await;
    let (store, _) = follower_with(&chain).await;
    let peer_head = &chain[2].header;

    assert_eq!(
        peer_head_on_local_chain(&store, Some(peer_head.number), peer_head.hash()).unwrap(),
        Some(3)
    );
    // eth/68 only advertises the hash.
    assert_eq!(
        peer_head_on_local_chain(&store, None, peer_head.hash()).unwrap(),
        Some(3)
    );
}

/// A peer ahead of us already has every block we could send.
#[tokio::test]
async fn broadcast_starts_at_a_peer_head_ahead_of_ours() {
    let chain = build_chain(5).await;
    let (store, _) = follower_with(&chain[..3]).await;
    let peer_head = &chain[4].header;

    assert_eq!(
        peer_head_on_local_chain(&store, Some(peer_head.number), peer_head.hash()).unwrap(),
        Some(5)
    );
}

/// A peer head that isn't on our chain gives no starting point.
#[tokio::test]
async fn broadcast_ignores_a_peer_head_off_our_chain() {
    let chain = build_chain(5).await;
    let other_chain = build_chain(3).await;
    let (store, _) = follower_with(&chain).await;
    let peer_head = &other_chain[2].header;

    assert_eq!(
        peer_head_on_local_chain(&store, Some(peer_head.number), peer_head.hash()).unwrap(),
        None
    );
    assert_eq!(
        peer_head_on_local_chain(&store, None, peer_head.hash()).unwrap(),
        None
    );
}

/// Blocks 1..=`length` produced on a separate store, as the sequencer would.
async fn build_chain(length: u64) -> Vec<Block> {
    let store = test_store().await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let mut parent = store.get_block_header(0).unwrap().unwrap();
    let mut chain = Vec::new();
    for _ in 0..length {
        let block = new_block(&store, &parent);
        parent = block.header.clone();
        import_block(&blockchain, &store, block.clone()).await;
        chain.push(block);
    }
    chain
}

async fn follower_with(blocks: &[Block]) -> (Store, Blockchain) {
    let store = test_store().await;
    let blockchain = Blockchain::default_with_store(store.clone());
    for block in blocks {
        import_block(&blockchain, &store, block.clone()).await;
    }
    (store, blockchain)
}

async fn import_block(blockchain: &Blockchain, store: &Store, block: Block) {
    let hash = block.hash();
    blockchain.add_block_pipeline(block, None).unwrap();
    apply_fork_choice(store, hash, hash, hash, None)
        .await
        .unwrap();
}

/// The state a connection starts with right after the `based` capability is negotiated.
async fn new_connection_state() -> L2ConnectedState {
    L2ConnectedState {
        latest_block_sent: 0,
        latest_batch_sent: 0,
        blocks_on_queue: BTreeMap::new(),
        batches_on_queue: BTreeMap::new(),
        store_rollup: StoreRollup::new(Path::new(""), EngineTypeRollup::InMemory).unwrap(),
        committer_key: Arc::new(SecretKey::from_slice(&[1; 32]).unwrap()),
        next_block_broadcast: Instant::now(),
        next_batch_broadcast: Instant::now(),
    }
}

fn queue(l2_state: &mut L2ConnectedState, block: &Block) {
    l2_state.blocks_on_queue.insert(
        block.header.number,
        QueuedBlock {
            block: Arc::new(block.clone()),
            fee_config: FeeConfig::default(),
        },
    );
}

fn peer() -> Node {
    Node::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 30303, 30303, H512::zero())
}

fn new_block(store: &Store, parent: &BlockHeader) -> Block {
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

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}
