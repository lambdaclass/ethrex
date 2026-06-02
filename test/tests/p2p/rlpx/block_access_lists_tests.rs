use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_p2p::rlpx::{
    eth::block_access_lists::{BlockAccessLists, OptionalBal},
    message::RLPxMessage,
};
use ethrex_rlp::encode::RLPEncode;

// ── BlockAccessLists (0x13, eth/71) ──
//
// EIP-8159 says a missing BAL is the RLP empty string (`0x80`), while a
// present-but-empty BAL (block with no state changes) is the RLP empty list
// (`0xc0`). The two must never alias, otherwise an upgraded node silently
// confuses "BAL unavailable" with "valid empty BAL" (interop break with geth).

#[test]
fn missing_bal_is_distinct_from_present_empty_bal() {
    let missing = BlockAccessLists::new(1, vec![None]);
    let empty = BlockAccessLists::new(1, vec![Some(BlockAccessList::from_accounts(vec![]))]);

    let mut missing_buf = Vec::new();
    missing.encode(&mut missing_buf).unwrap();
    let mut empty_buf = Vec::new();
    empty.encode(&mut empty_buf).unwrap();

    // 0x80 sentinel must not collapse onto the 0xc0 empty-list encoding.
    assert_ne!(missing_buf, empty_buf);
}

#[test]
fn missing_bal_roundtrips_as_none() {
    let msg = BlockAccessLists::new(7, vec![None]);
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();

    let decoded = BlockAccessLists::decode(&buf).unwrap();
    assert_eq!(decoded.id, 7);
    assert_eq!(decoded.block_access_lists.len(), 1);
    assert!(decoded.block_access_lists[0].is_none());
}

#[test]
fn present_empty_bal_roundtrips_as_some() {
    let msg = BlockAccessLists::new(7, vec![Some(BlockAccessList::from_accounts(vec![]))]);
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();

    let decoded = BlockAccessLists::decode(&buf).unwrap();
    assert_eq!(decoded.block_access_lists.len(), 1);
    assert!(decoded.block_access_lists[0].is_some());
}

/// Locks the EIP-8159 §"BlockAccessLists (0x13)" sentinel: a missing BAL
/// encodes as exactly the RLP empty string (`0x80`), never the empty list
/// (`0xc0`, a valid empty BAL). geth uses the same sentinel (`rlp.EmptyString`
/// in `eth/protocols/eth/handlers.go`); any drift here is silent interop
/// breakage. Asserts the raw byte directly on the `OptionalBal` wrapper, which
/// the message-level tests can't see (their bytes go through snappy).
#[test]
fn optional_bal_none_encodes_as_0x80_sentinel() {
    let mut bytes = Vec::new();
    OptionalBal(None).encode(&mut bytes);
    assert_eq!(bytes, vec![0x80]);
}
