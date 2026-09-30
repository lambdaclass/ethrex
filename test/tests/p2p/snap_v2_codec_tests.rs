//! snap/2 codec round-trip tests (EIP-8189).
//!
//! Exercises `Snap2GetBlockAccessLists` / `Snap2BlockAccessLists` encode/decode
//! plus the spec-mandated `0x80` None sentinel (§50, §58).

use ethrex_common::{H256, types::block_access_list::BlockAccessList};
use ethrex_p2p::rlpx::message::RLPxMessage;
use ethrex_p2p::rlpx::snap::{Snap2BlockAccessLists, Snap2GetBlockAccessLists};
use ethrex_p2p::rlpx::utils::snappy_decompress;

fn sample_bal() -> BlockAccessList {
    BlockAccessList::default()
}

fn roundtrip_get_bal(msg: Snap2GetBlockAccessLists) -> Snap2GetBlockAccessLists {
    let mut buf = vec![];
    msg.encode(&mut buf).expect("encode");
    Snap2GetBlockAccessLists::decode(&buf).expect("decode")
}

fn roundtrip_bal(msg: Snap2BlockAccessLists) -> Snap2BlockAccessLists {
    let mut buf = vec![];
    msg.encode(&mut buf).expect("encode");
    Snap2BlockAccessLists::decode(&buf).expect("decode")
}

#[test]
fn snap2_get_bal_empty_roundtrip() {
    let msg = Snap2GetBlockAccessLists {
        id: 1,
        block_hashes: vec![],
        response_bytes: 0,
    };
    let decoded = roundtrip_get_bal(msg);
    assert_eq!(decoded.id, 1);
    assert!(decoded.block_hashes.is_empty());
    assert_eq!(decoded.response_bytes, 0);
}

#[test]
fn snap2_get_bal_with_hashes_roundtrip() {
    let hashes = vec![H256::from([1u8; 32]), H256::from([2u8; 32])];
    let msg = Snap2GetBlockAccessLists {
        id: 42,
        block_hashes: hashes.clone(),
        response_bytes: 1024,
    };
    let decoded = roundtrip_get_bal(msg);
    assert_eq!(decoded.id, 42);
    assert_eq!(decoded.block_hashes, hashes);
    assert_eq!(decoded.response_bytes, 1024);
}

#[test]
fn snap2_bal_empty_roundtrip() {
    let msg = Snap2BlockAccessLists {
        id: 9,
        bals: vec![],
    };
    let decoded = roundtrip_bal(msg);
    assert_eq!(decoded.id, 9);
    assert!(decoded.bals.is_empty());
}

#[test]
fn snap2_bal_all_none_roundtrip() {
    let msg = Snap2BlockAccessLists {
        id: 5,
        bals: vec![None, None, None],
    };
    let decoded = roundtrip_bal(msg);
    assert_eq!(decoded.id, 5);
    assert_eq!(decoded.bals.len(), 3);
    assert!(decoded.bals.iter().all(|b| b.is_none()));
}

#[test]
fn snap2_bal_all_some_roundtrip() {
    let msg = Snap2BlockAccessLists {
        id: 7,
        bals: vec![Some(sample_bal()), Some(sample_bal())],
    };
    let decoded = roundtrip_bal(msg);
    assert_eq!(decoded.id, 7);
    assert_eq!(decoded.bals.len(), 2);
    assert!(decoded.bals.iter().all(|b| b.is_some()));
}

#[test]
fn snap2_bal_mixed_roundtrip() {
    let msg = Snap2BlockAccessLists {
        id: 11,
        bals: vec![Some(sample_bal()), None, Some(sample_bal()), None],
    };
    let decoded = roundtrip_bal(msg);
    assert_eq!(decoded.id, 11);
    assert_eq!(decoded.bals.len(), 4);
    assert!(decoded.bals[0].is_some());
    assert!(decoded.bals[1].is_none());
    assert!(decoded.bals[2].is_some());
    assert!(decoded.bals[3].is_none());
}

#[test]
fn snap2_bal_none_uses_0x80_sentinel() {
    // Locks EIP-8189 §50/§58: None encodes to RLP empty string (0x80),
    // not the eth/71 empty-list (0xc0). Verified via the decoded snappy
    // payload — `0x80` must be present and `0xc0` absent when encoding
    // a single-None response.
    let msg = Snap2BlockAccessLists {
        id: 0,
        bals: vec![None],
    };
    let mut buf = vec![];
    msg.encode(&mut buf).expect("encode");
    let decompressed = snappy_decompress(&buf).expect("decompress");
    assert!(
        decompressed.contains(&0x80),
        "decompressed payload must contain the 0x80 None sentinel"
    );
    assert!(
        !decompressed.contains(&0xc0),
        "decompressed payload must not contain the eth/71 0xc0 empty-list sentinel"
    );
}
