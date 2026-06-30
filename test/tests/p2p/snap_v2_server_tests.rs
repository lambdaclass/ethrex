//! snap/2 server handler unit tests (EIP-8189).
//!
//! These tests call `build_snap2_bal_response` directly to validate the handler
//! logic without spinning up two RLPx peers.

use ethrex_common::{H256, types::BlockHeader, types::block_access_list::BlockAccessList};
use ethrex_p2p::rlpx::connection::server::build_snap2_bal_response;
use ethrex_p2p::rlpx::snap::Snap2GetBlockAccessLists;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store, api::tables::HEADERS};

fn make_store() -> Store {
    Store::new("memory", EngineType::InMemory).expect("in-memory store")
}

fn make_req(hashes: Vec<H256>, response_bytes: u64) -> Snap2GetBlockAccessLists {
    Snap2GetBlockAccessLists {
        id: 1,
        block_hashes: hashes,
        response_bytes,
    }
}

/// Store a header for the given hash using the same encoding as the production path.
/// Uses the synchronous `Store::write()` to avoid async complexity in tests.
fn store_header(store: &Store, hash: H256, header: BlockHeader) {
    use ethrex_storage::rlp::BlockHeaderRLP;
    let hash_key = hash.encode_to_vec();
    let header_bytes = BlockHeaderRLP::from(header).into_vec();
    store
        .write(HEADERS, hash_key, header_bytes)
        .expect("store header");
}

/// Build a post-Amsterdam block header.
///
/// A post-Amsterdam header must have all prior optional fields present so that
/// RLP encoding/decoding correctly positions `block_access_list_hash`. Fields
/// introduced before Amsterdam (Cancun: blob_gas_used, excess_blob_gas,
/// parent_beacon_block_root; Prague: requests_hash) must all be Some.
fn post_amsterdam_header() -> BlockHeader {
    BlockHeader {
        base_fee_per_gas: Some(0),
        withdrawals_root: Some(H256::zero()),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(H256::zero()),
        block_access_list_hash: Some(H256::from([0xBBu8; 32])),
        ..Default::default()
    }
}

/// Store a post-Amsterdam header (has `block_access_list_hash: Some(...)`).
fn store_post_amsterdam_header(store: &Store, hash: H256) {
    store_header(store, hash, post_amsterdam_header());
}

/// Store a pre-Amsterdam header (has `block_access_list_hash: None`).
fn store_pre_amsterdam_header(store: &Store, hash: H256) {
    let header = BlockHeader {
        ..Default::default()
    };
    store_header(store, hash, header);
}

#[test]
fn snap2_server_returns_empty_list_for_empty_request() {
    let store = make_store();
    let req = make_req(vec![], 0);
    let resp = build_snap2_bal_response(req, &store).expect("should succeed");
    assert_eq!(resp.id, 1);
    assert!(resp.bals.is_empty(), "empty request → empty response");
}

#[test]
fn snap2_server_returns_none_for_unknown_hash() {
    let store = make_store();
    let hash = H256::from([0xABu8; 32]);
    let req = make_req(vec![hash], 0);
    let resp = build_snap2_bal_response(req, &store).expect("should succeed");
    assert_eq!(resp.bals.len(), 1);
    assert!(resp.bals[0].is_none(), "unknown hash should return None");
}

#[test]
fn snap2_server_returns_none_for_pre_amsterdam_header() {
    let store = make_store();
    let hash = H256::from([0x11u8; 32]);
    store_pre_amsterdam_header(&store, hash);

    let req = make_req(vec![hash], 0);
    let resp = build_snap2_bal_response(req, &store).expect("should succeed");
    assert_eq!(resp.bals.len(), 1);
    assert!(
        resp.bals[0].is_none(),
        "pre-Amsterdam header (no block_access_list_hash) should return None"
    );
}

#[test]
fn snap2_server_returns_some_for_known_hash() {
    let store = make_store();
    let hash = H256::from([0x22u8; 32]);
    store_post_amsterdam_header(&store, hash);

    let bal = BlockAccessList::new();
    store
        .store_block_access_list(hash, &bal)
        .expect("store BAL");

    let req = make_req(vec![hash], 0);
    let resp = build_snap2_bal_response(req, &store).expect("should succeed");
    assert_eq!(resp.bals.len(), 1);
    assert!(
        resp.bals[0].is_some(),
        "known post-Amsterdam hash with stored BAL should return Some"
    );
}

#[test]
fn snap2_server_uses_2mib_default_when_response_bytes_zero() {
    // When response_bytes == 0, the cap should be 2 MiB (BAL_RESPONSE_SOFT_CAP_BYTES).
    // We verify indirectly: a small empty BAL with response_bytes=0 must still be served.
    let store = make_store();
    let hash = H256::from([0x33u8; 32]);
    store_post_amsterdam_header(&store, hash);
    let bal = BlockAccessList::new();
    store
        .store_block_access_list(hash, &bal)
        .expect("store BAL");

    let req = make_req(vec![hash], 0);
    let resp = build_snap2_bal_response(req, &store).expect("should succeed");
    // Should serve the BAL (cap is 2 MiB, not 0).
    assert_eq!(resp.bals.len(), 1);
    assert!(
        resp.bals[0].is_some(),
        "BAL should be served when response_bytes=0"
    );
}

#[test]
fn snap2_server_truncates_from_tail_on_size_cap() {
    // Set a very small cap (response_bytes = 1) so only the first entry fits.
    // The server must include at least 1 entry (§51) and then stop once cap exceeded.
    let store = make_store();

    let hash_a = H256::from([0x44u8; 32]);
    let hash_b = H256::from([0x55u8; 32]);
    let hash_c = H256::from([0x66u8; 32]);

    for hash in [hash_a, hash_b, hash_c] {
        store_post_amsterdam_header(&store, hash);
        store
            .store_block_access_list(hash, &BlockAccessList::new())
            .expect("store BAL");
    }

    // response_bytes = 1 forces truncation after the first entry.
    let req = make_req(vec![hash_a, hash_b, hash_c], 1);
    let resp = build_snap2_bal_response(req, &store).expect("should succeed");

    // Must include at least 1 entry (the first one) but not all 3.
    assert!(
        !resp.bals.is_empty(),
        "must include at least 1 entry even when cap is tiny"
    );
    assert!(
        resp.bals.len() < 3,
        "should truncate from tail when cap is exceeded"
    );
    assert!(
        resp.bals[0].is_some(),
        "first entry should be served even under tight cap"
    );
}

#[test]
fn snap2_server_caps_excess_hashes_to_max_request_size() {
    // EIP-8189 §51 + DoS defense: cap per-request hash list at
    // `BAL_MAX_REQUEST_HASHES` (1024, matching geth's `maxAccessListLookups`).
    // A request with more hashes must produce at most `BAL_MAX_REQUEST_HASHES`
    // slots in the response.
    use ethrex_p2p::snap::constants::BAL_MAX_REQUEST_HASHES;

    let store = make_store();
    let mut hashes = Vec::with_capacity(BAL_MAX_REQUEST_HASHES + 5);
    for i in 0..(BAL_MAX_REQUEST_HASHES + 5) {
        hashes.push(H256::from_low_u64_be(i as u64));
    }

    let req = make_req(hashes, 0);
    let resp = build_snap2_bal_response(req, &store).expect("should succeed");
    assert!(
        resp.bals.len() <= BAL_MAX_REQUEST_HASHES,
        "response must not exceed BAL_MAX_REQUEST_HASHES entries (got {})",
        resp.bals.len()
    );
}

#[test]
fn snap2_server_returns_none_for_post_amsterdam_header_without_bal() {
    // §50: the response slot must be `None` (encoded as 0x80) even when the
    // header itself is post-Amsterdam but no BAL is currently stored locally.
    // This is distinct from §100 (pre-Amsterdam header → None unconditionally).
    let store = make_store();
    let hash = H256::from([0x77u8; 32]);
    store_post_amsterdam_header(&store, hash);
    // Deliberately do NOT call store_block_access_list — header exists, BAL doesn't.

    let req = make_req(vec![hash], 0);
    let resp = build_snap2_bal_response(req, &store).expect("should succeed");
    assert_eq!(resp.bals.len(), 1);
    assert!(
        resp.bals[0].is_none(),
        "post-Amsterdam header with no stored BAL must yield None"
    );
}
