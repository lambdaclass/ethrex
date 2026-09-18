use ethrex_common::H256;
use ethrex_common::types::{Block, BlockBody, BlockHash, BlockHeader};
use ethrex_p2p::rlpx::{
    eth::blocks::{BlockBodies, GetBlockBodies, GetBlockHeaders, HashOrNumber},
    message::RLPxMessage,
};
use ethrex_storage::{EngineType, Store};

#[test]
fn get_block_headers_startblock_number_message() {
    let get_block_bodies = GetBlockHeaders::new(1, HashOrNumber::Number(1), 0, 0, false);

    let mut buf = Vec::new();
    get_block_bodies.encode(&mut buf).unwrap();

    let decoded = GetBlockHeaders::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.startblock, HashOrNumber::Number(1));
}

#[test]
fn get_block_headers_startblock_hash_message() {
    let get_block_bodies =
        GetBlockHeaders::new(1, HashOrNumber::Hash(BlockHash::from([1; 32])), 0, 0, false);

    let mut buf = Vec::new();
    get_block_bodies.encode(&mut buf).unwrap();

    let decoded = GetBlockHeaders::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(
        decoded.startblock,
        HashOrNumber::Hash(BlockHash::from([1; 32]))
    );
}

// A child of `parent` at `number`, its hash varied via `timestamp`. Optional
// trailing fields are set so the header round-trips through RLP.
fn child(number: u64, timestamp: u64, parent: BlockHash) -> BlockHeader {
    BlockHeader {
        number,
        timestamp,
        parent_hash: parent,
        base_fee_per_gas: Some(0),
        withdrawals_root: Some(H256::zero()),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(H256::zero()),
        ..Default::default()
    }
}

async fn store_block(store: &Store, header: BlockHeader) -> BlockHash {
    let hash = header.hash();
    store
        .add_block(Block {
            header,
            body: BlockBody::default(),
        })
        .await
        .expect("store block");
    hash
}

// A syncing peer asks for its fork/sync head by hash. That head is not canonical
// on the serving node, so the old code translated hash -> number -> the canonical
// block at that height and answered with a DIFFERENT block; the peer then rejected
// the whole batch. GetBlockHeaders must serve the block at the requested hash
// itself, and walk parent hashes for a reverse-by-hash request.
#[tokio::test]
async fn get_block_headers_serves_a_non_canonical_head_by_hash() {
    let store = Store::new("", EngineType::InMemory).expect("store");

    // Canonical chain 0 <- 1 <- 2c.
    let genesis = store_block(&store, child(0, 1, BlockHash::zero())).await;
    let block1 = store_block(&store, child(1, 1, genesis)).await;
    let canonical2 = store_block(&store, child(2, 1, block1)).await;
    // A non-canonical sibling at height 2, and a fork head at height 3 on top of it.
    let fork2 = store_block(&store, child(2, 2, block1)).await;
    let fork_head = store_block(&store, child(3, 2, fork2)).await;
    store
        .forkchoice_update(
            vec![(0, genesis), (1, block1), (2, canonical2)],
            2,
            canonical2,
            None,
            None,
        )
        .await
        .expect("make canonical");
    assert_ne!(fork2, canonical2);

    // Single header by the non-canonical hash: must be that block, not canonical2.
    let got = GetBlockHeaders::new(1, HashOrNumber::Hash(fork2), 1, 0, false)
        .fetch_headers(&store)
        .await;
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].hash(),
        fork2,
        "must serve the requested non-canonical block"
    );

    // Reverse-by-hash from the fork head walks parent hashes down its own branch,
    // not the canonical chain.
    let got = GetBlockHeaders::new(2, HashOrNumber::Hash(fork_head), 3, 0, true)
        .fetch_headers(&store)
        .await;
    let chain: Vec<BlockHash> = got.iter().map(|h| h.hash()).collect();
    assert_eq!(chain, vec![fork_head, fork2, block1]);
}

#[test]
fn get_block_bodies_empty_message() {
    let blocks_hash = vec![];
    let get_block_bodies = GetBlockBodies::new(1, blocks_hash.clone());

    let mut buf = Vec::new();
    get_block_bodies.encode(&mut buf).unwrap();

    let decoded = GetBlockBodies::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.block_hashes, blocks_hash);
}

#[test]
fn get_block_bodies_not_empty_message() {
    let blocks_hash = vec![
        BlockHash::from([0; 32]),
        BlockHash::from([1; 32]),
        BlockHash::from([2; 32]),
    ];
    let get_block_bodies = GetBlockBodies::new(1, blocks_hash.clone());

    let mut buf = Vec::new();
    get_block_bodies.encode(&mut buf).unwrap();

    let decoded = GetBlockBodies::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.block_hashes, blocks_hash);
}

#[test]
fn block_bodies_empty_message() {
    let block_bodies = vec![];
    let block_bodies = BlockBodies::new(1, block_bodies);

    let mut buf = Vec::new();
    block_bodies.encode(&mut buf).unwrap();

    let decoded = BlockBodies::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.block_bodies, vec![]);
}
