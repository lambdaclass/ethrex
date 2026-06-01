use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_p2p::rlpx::{eth::block_access_lists::BlockAccessLists, message::RLPxMessage};

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
