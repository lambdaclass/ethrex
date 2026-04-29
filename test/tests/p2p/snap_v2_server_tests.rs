//! snap/2 (EIP-8189) BAL server tests — Task 8.2 / 8.3
//!
//! # Test-infra audit (Task 8.0)
//!
//! ## Harness pattern (from snap_server_tests.rs)
//!
//! All existing snap server tests are **direct unit tests**: they create an
//! in-memory `Store`, insert state data, then call server functions
//! (`process_account_range_request`, etc.) directly without any network stack.
//! No multi-node or networked harness exists in `test/tests/p2p/`.
//!
//! ## snap/2 requirements for direct server tests (Tasks 8.2 / 8.3)
//!
//! - An in-memory `Store` is sufficient; `store_block_access_list` / `get_block_access_list`
//!   are used to pre-populate BALs and `process_block_access_lists_request` is called directly.
//! - No Amsterdam fork activation flag is needed for direct server calls — the
//!   fork gate is enforced by the caller (`advance_state_via_bals`), not by the
//!   server handler itself.
//! - `SnapCapVersion` is not needed here either: the Message dispatch layer
//!   (tested via 8.3) handles the version check before the server is reached.
//!
//! ## End-to-end snap/2 tests (Tasks 8.5 / 8.6 / 8.7 — infeasibility notice)
//!
//! Tasks 8.5 and 8.6 require spinning up two ethrex nodes that negotiate
//! snap/2, advancing the source by N blocks producing BALs, and syncing
//! the second node end-to-end.  This requires a multi-node network harness
//! with a live RLPx connection stack.  No such harness exists in
//! `test/tests/p2p/` or in `crates/networking/p2p/`; the only integration
//! test environment is hive (external).
//!
//! Conclusion: Tasks 8.5 and 8.6 are **infeasible** without building a new
//! multi-node harness from scratch, which the plan explicitly forbids
//! ("do not invent a parallel harness from scratch").  They are surfaced here
//! as an explicit infeasibility item.
//!
//! Task 8.7 (malicious peer) is covered as a unit test in
//! `crates/networking/p2p/sync/bal_healing/mod.rs` (see below) instead of
//! requiring a full integration harness.

use ethrex_common::{
    Address, H256, U256,
    types::block_access_list::{AccountChanges, BalanceChange, BlockAccessList},
};
use ethrex_p2p::{
    rlpx::snap::GetBlockAccessLists,
    snap::{SnapError, process_block_access_lists_request},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn empty_store() -> Store {
    Store::new("null", EngineType::InMemory).unwrap()
}

/// Build a BAL with a single account having a given balance, for size testing.
fn make_bal_with_balance(addr_byte: u8, balance: u64) -> BlockAccessList {
    let mut bal = BlockAccessList::new();
    let mut changes = AccountChanges::new(Address::from([addr_byte; 20]));
    changes.add_balance_change(BalanceChange::new(0, U256::from(balance)));
    bal.add_account_changes(changes);
    bal
}

/// Store a BAL and return its hash.
fn store_bal(store: &Store, block_hash: H256, bal: &BlockAccessList) {
    store.store_block_access_list(block_hash, bal).unwrap();
}

// ─── Task 8.2a: empty request ─────────────────────────────────────────────

#[tokio::test]
async fn bal_server_empty_request() -> Result<(), SnapError> {
    let store = empty_store();
    let request = GetBlockAccessLists {
        id: 1,
        block_hashes: vec![],
        response_bytes: 2 * 1024 * 1024,
    };
    let resp = process_block_access_lists_request(request, store).await?;
    assert_eq!(resp.id, 1);
    assert!(resp.bals.is_empty());
    Ok(())
}

// ─── Task 8.2b: all-known hashes ──────────────────────────────────────────

#[tokio::test]
async fn bal_server_all_known() -> Result<(), SnapError> {
    let store = empty_store();
    let hashes: Vec<H256> = (0u8..3).map(|i| H256::from([i + 1; 32])).collect();
    let bals: Vec<BlockAccessList> = (0u8..3).map(|i| make_bal_with_balance(i, 100)).collect();

    for (hash, bal) in hashes.iter().zip(bals.iter()) {
        store_bal(&store, *hash, bal);
    }

    let request = GetBlockAccessLists {
        id: 2,
        block_hashes: hashes.clone(),
        response_bytes: 2 * 1024 * 1024,
    };
    let resp = process_block_access_lists_request(request, store).await?;
    assert_eq!(resp.id, 2);
    assert_eq!(resp.bals.len(), 3);
    // All positions should be Some.
    for item in &resp.bals {
        assert!(item.is_some(), "expected Some for known hash");
    }
    Ok(())
}

// ─── Task 8.2c: mix of known and unknown hashes ───────────────────────────

#[tokio::test]
async fn bal_server_mixed_known_unknown() -> Result<(), SnapError> {
    let store = empty_store();
    // Use 4 hashes: store only for indices 0 and 2.
    let hashes: Vec<H256> = (0u8..4).map(|i| H256::from([i + 10; 32])).collect();
    store_bal(&store, hashes[0], &make_bal_with_balance(0xA0, 1));
    store_bal(&store, hashes[2], &make_bal_with_balance(0xA2, 2));

    let request = GetBlockAccessLists {
        id: 3,
        block_hashes: hashes,
        response_bytes: 2 * 1024 * 1024,
    };
    let resp = process_block_access_lists_request(request, store).await?;
    assert_eq!(resp.bals.len(), 4);
    assert!(resp.bals[0].is_some(), "idx 0 should be Some (known)");
    assert!(resp.bals[1].is_none(), "idx 1 should be None (unknown)");
    assert!(resp.bals[2].is_some(), "idx 2 should be Some (known)");
    assert!(resp.bals[3].is_none(), "idx 3 should be None (unknown)");
    Ok(())
}

// ─── Task 8.2d: cumulative size cap with first-slot-always-included ────────

#[tokio::test]
async fn bal_server_size_cap_first_always_included() -> Result<(), SnapError> {
    let store = empty_store();

    // Build a large BAL that exceeds the 2 MiB cap by itself.
    // We simulate this by storing 3 BALs and setting response_bytes = 1 byte,
    // which means the second and third entries should be None (cap reached after
    // first entry is included — soft cap: first entry is always included even if
    // it alone exceeds the cap).
    let hashes: Vec<H256> = (0u8..3).map(|i| H256::from([i + 20; 32])).collect();
    for (i, hash) in hashes.iter().enumerate() {
        store_bal(&store, *hash, &make_bal_with_balance(i as u8, 1000));
    }

    // response_bytes = 1 forces the cap to be 1 byte; first BAL is still returned.
    let request = GetBlockAccessLists {
        id: 4,
        block_hashes: hashes.clone(),
        response_bytes: 1,
    };
    let resp = process_block_access_lists_request(request, store.clone()).await?;
    assert_eq!(
        resp.bals.len(),
        3,
        "position correspondence: all 3 slots returned"
    );
    assert!(
        resp.bals[0].is_some(),
        "first slot always included even when cap = 1"
    );
    // Remaining slots should be None because cap was reached.
    assert!(resp.bals[1].is_none(), "slot 1 should be capped out");
    assert!(resp.bals[2].is_none(), "slot 2 should be capped out");

    // Also verify the normal cap: response_bytes = encoded size of first BAL ensures
    // the second BAL will push over the cap.
    let bal0 = make_bal_with_balance(0, 1000);
    let first_size = bal0.encode_to_vec().len() as u64;

    let request2 = GetBlockAccessLists {
        id: 5,
        block_hashes: hashes,
        response_bytes: first_size, // exactly enough for one BAL
    };
    let resp2 = process_block_access_lists_request(request2, store).await?;
    assert_eq!(resp2.bals.len(), 3);
    assert!(resp2.bals[0].is_some(), "first BAL included");
    // Second BAL pushes over cap — should be None.
    assert!(resp2.bals[1].is_none(), "second BAL should be capped");
    assert!(resp2.bals[2].is_none(), "third BAL should be capped");
    Ok(())
}

// ─── Task 8.3: snap/1 connection must reject GetBlockAccessLists ──────────
//
// This is a regression test confirming that `Message::decode` returns
// `MalformedData` when a snap/1 connection receives a GetBlockAccessLists
// frame (message code 0x08 within the snap cap range).
//
// The server's `process_block_access_lists_request` is never reached on a
// snap/1 connection because the dispatch layer rejects the frame before
// routing to any handler.

#[test]
fn snap1_rejects_get_block_access_lists() {
    use ethrex_p2p::rlpx::{
        message::{EthCapVersion, Message, RLPxMessage, SnapCapVersion},
        snap::GetBlockAccessLists as MsgGetBAL,
    };
    use ethrex_rlp::{encode::RLPEncode, error::RLPDecodeError};

    // Build a valid GetBlockAccessLists wire payload.
    let msg = MsgGetBAL {
        id: 99,
        block_hashes: vec![H256::from([0xAB; 32])],
        response_bytes: 1024,
    };
    let mut buf = vec![];
    msg.encode(&mut buf).unwrap();

    // The snap offset for ETH/68 is 0x21; GetBlockAccessLists code is 0x08.
    // Raw wire id = 0x21 + 0x08 = 0x29.
    const SNAP_OFFSET_ETH68: u8 = 0x21; // from EthCapVersion::V68.snap_capability_offset()
    let raw_id = SNAP_OFFSET_ETH68 + 0x08u8;

    // Under snap/1 this must be rejected.
    let result = Message::decode(raw_id, &buf, EthCapVersion::V68, SnapCapVersion::V1);
    assert!(
        matches!(result, Err(RLPDecodeError::MalformedData)),
        "snap/1 must reject GetBlockAccessLists (0x08), got: {result:?}"
    );

    // Under snap/2 it must succeed.
    let result_v2 = Message::decode(raw_id, &buf, EthCapVersion::V68, SnapCapVersion::V2);
    assert!(
        result_v2.is_ok(),
        "snap/2 must accept GetBlockAccessLists, got: {result_v2:?}"
    );

    // Also confirm that GetTrieNodes (0x06) under snap/2 is rejected.
    use ethrex_p2p::rlpx::snap::GetTrieNodes;
    let trie_msg = GetTrieNodes {
        id: 1,
        root_hash: H256::zero(),
        paths: vec![],
        bytes: 1024,
    };
    let mut trie_buf = vec![];
    trie_msg.encode(&mut trie_buf).unwrap();
    let trie_raw_id = SNAP_OFFSET_ETH68 + 0x06u8;

    let result_v2_trie = Message::decode(
        trie_raw_id,
        &trie_buf,
        EthCapVersion::V68,
        SnapCapVersion::V2,
    );
    assert!(
        matches!(result_v2_trie, Err(RLPDecodeError::MalformedData)),
        "snap/2 must reject GetTrieNodes (0x06), got: {result_v2_trie:?}"
    );
}
