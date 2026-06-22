use ethrex_common::types::BlockHash;
use ethrex_p2p::rlpx::{
    eth::blocks::{BlockBodies, GetBlockBodies, GetBlockHeaders, HashOrNumber},
    message::RLPxMessage,
};

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
