//! EIP-8282 request-encoding and hashing tests (no VM).
//!
//! These exercise the `Requests` enum's new builder-deposit (0x03) and
//! builder-exit (0x04) variants and how `compute_requests_hash` commits to
//! them: the type byte prefix, exclusion of empty requests, and ordering.

use ethrex_common::types::requests::{Requests, compute_requests_hash};

// Builds the three pre-Amsterdam request entries (deposit, withdrawal,
// consolidation) with distinct non-empty data so the prefix hash is stable.
fn three_request_prefix() -> Vec<ethrex_common::types::requests::EncodedRequests> {
    vec![
        Requests::Deposit(vec![]).encode(),
        Requests::from_withdrawals_data(vec![0x11, 0x22]).encode(),
        Requests::from_consolidation_data(vec![0x33, 0x44]).encode(),
    ]
}

#[test]
fn builder_deposit_encode_prepends_0x03() {
    let encoded = Requests::BuilderDeposit(vec![0xaa, 0xbb]).encode();
    assert_eq!(
        encoded.0.first().copied(),
        Some(0x03),
        "builder-deposit request must be tagged with type byte 0x03"
    );
    assert_eq!(encoded.0.as_ref(), &[0x03, 0xaa, 0xbb]);
}

#[test]
fn builder_exit_encode_prepends_0x04() {
    let encoded = Requests::BuilderExit(vec![0xcc, 0xdd]).encode();
    assert_eq!(
        encoded.0.first().copied(),
        Some(0x04),
        "builder-exit request must be tagged with type byte 0x04"
    );
    assert_eq!(encoded.0.as_ref(), &[0x04, 0xcc, 0xdd]);
}

#[test]
fn nonempty_builder_requests_change_the_hash() {
    let prefix_hash = compute_requests_hash(&three_request_prefix());

    let mut with_builders = three_request_prefix();
    with_builders.push(Requests::from_builder_deposit_data(vec![0x01, 0x02, 0x03]).encode());
    with_builders.push(Requests::from_builder_exit_data(vec![0x04, 0x05, 0x06]).encode());
    let with_builders_hash = compute_requests_hash(&with_builders);

    assert_ne!(
        prefix_hash, with_builders_hash,
        "appending non-empty builder-deposit/builder-exit requests must change the requests hash"
    );
}

#[test]
fn empty_builder_requests_are_excluded_from_the_hash() {
    let prefix_hash = compute_requests_hash(&three_request_prefix());

    // Empty builder data => encoded length is 1 (just the type byte), which
    // compute_requests_hash skips. The 5-element hash must equal the 3-element one.
    let mut with_empty_builders = three_request_prefix();
    with_empty_builders.push(Requests::from_builder_deposit_data(vec![]).encode());
    with_empty_builders.push(Requests::from_builder_exit_data(vec![]).encode());
    let with_empty_builders_hash = compute_requests_hash(&with_empty_builders);

    assert_eq!(
        prefix_hash, with_empty_builders_hash,
        "empty builder requests (encoded len <= 1) must be excluded from the requests hash"
    );
}

#[test]
fn builder_request_ordering_is_committed() {
    let deposit_data = vec![0x01, 0x02, 0x03];
    let exit_data = vec![0x04, 0x05, 0x06];

    let mut canonical = three_request_prefix();
    canonical.push(Requests::from_builder_deposit_data(deposit_data.clone()).encode());
    canonical.push(Requests::from_builder_exit_data(exit_data.clone()).encode());
    let canonical_hash = compute_requests_hash(&canonical);

    // Swap the builder-deposit and builder-exit entries. Same bytes, different order.
    let mut swapped = three_request_prefix();
    swapped.push(Requests::from_builder_exit_data(exit_data).encode());
    swapped.push(Requests::from_builder_deposit_data(deposit_data).encode());
    let swapped_hash = compute_requests_hash(&swapped);

    assert_ne!(
        canonical_hash, swapped_hash,
        "swapping the builder-deposit and builder-exit entries must change the requests hash"
    );
}
