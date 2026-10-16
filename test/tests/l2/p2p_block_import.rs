//! Block sync over the L2 `based` capability.
//!
//! A follower must keep importing new blocks whatever connections deliver them: after the
//! sequencer or the follower restarts, and when it holds several connections at once. The
//! sequencer should only re-send what the follower is missing.

use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, fee_config::FeeConfig,
    },
};
use ethrex_p2p::rlpx::l2::{
    block_importer::L2BlockImporter, l2_connection::peer_head_on_local_chain,
};
use ethrex_storage::{EngineType, Store};
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};

/// The follower already has blocks 1..=5, as after the sequencer or the follower restarts. It
/// imports 6 and 7 as they arrive, whatever the order.
#[tokio::test]
async fn importer_extends_the_local_head() {
    let sequencer_chain = build_chain(7).await;
    let (store, blockchain) = follower_with(&sequencer_chain[..5]).await;
    let store_rollup = rollup_store();
    let importer = L2BlockImporter::default();

    // Block 7 arrives first: it has to wait for 6.
    assert!(submit(&importer, &sequencer_chain[6]));
    importer
        .import_ready_blocks(&store, &blockchain, &store_rollup)
        .await
        .unwrap();
    assert_eq!(store.get_latest_block_number().unwrap(), 5);
    assert!(importer.is_queued(7));

    assert!(submit(&importer, &sequencer_chain[5]));
    importer
        .import_ready_blocks(&store, &blockchain, &store_rollup)
        .await
        .unwrap();
    assert_eq!(store.get_latest_block_number().unwrap(), 7);
    assert_eq!(
        store.get_block_header(7).unwrap().unwrap().hash(),
        sequencer_chain[6].hash()
    );
    assert!(!importer.is_queued(7));
}

/// Blocks at or below the head, e.g. stored some other way since they were queued, are dropped
/// instead of blocking the queue.
#[tokio::test]
async fn blocks_at_or_below_the_head_are_dropped() {
    let sequencer_chain = build_chain(7).await;
    let (store, blockchain) = follower_with(&sequencer_chain[..5]).await;
    let importer = L2BlockImporter::default();

    for block in &sequencer_chain[2..] {
        submit(&importer, block);
    }
    import_block(&blockchain, &store, sequencer_chain[5].clone()).await;

    importer
        .import_ready_blocks(&store, &blockchain, &rollup_store())
        .await
        .unwrap();
    assert_eq!(store.get_latest_block_number().unwrap(), 7);
    assert!((3..=7).all(|number| !importer.is_queued(number)));
}

/// A block delivered again while it's still queued, e.g. by another connection, isn't queued
/// twice, so the connection that got the duplicate doesn't re-broadcast it.
#[tokio::test]
async fn a_queued_block_is_not_queued_again() {
    let sequencer_chain = build_chain(1).await;
    let importer = L2BlockImporter::default();

    assert!(submit(&importer, &sequencer_chain[0]));
    assert!(!submit(&importer, &sequencer_chain[0]));
}

/// Several connections delivering the same blocks at once, as when the node holds more than one
/// connection to the sequencer. The blocks get imported without overlapping imports, which would
/// deadlock on the merkleization pool.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocks_from_several_connections_are_imported() {
    let sequencer_chain = build_chain(10).await;
    let (store, blockchain) = follower_with(&[]).await;
    let importer = L2BlockImporter::spawn(store.clone(), blockchain, rollup_store());

    for _ in 0..4 {
        let (importer, blocks) = (importer.clone(), sequencer_chain.clone());
        tokio::spawn(async move {
            for block in &blocks {
                submit(&importer, block);
                tokio::task::yield_now().await;
            }
        });
    }
    tokio::time::timeout(Duration::from_secs(60), async {
        while store.get_latest_block_number().unwrap() < 10 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("blocks were not imported");
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

async fn follower_with(blocks: &[Block]) -> (Store, Arc<Blockchain>) {
    let store = test_store().await;
    let blockchain = Arc::new(Blockchain::default_with_store(store.clone()));
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

fn submit(importer: &L2BlockImporter, block: &Block) -> bool {
    importer.submit(Arc::new(block.clone()), FeeConfig::default())
}

fn rollup_store() -> StoreRollup {
    StoreRollup::new(Path::new(""), EngineTypeRollup::InMemory).unwrap()
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
