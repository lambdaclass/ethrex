//! `debug_getRawReceipts` must not present a block's receipts as empty when they
//! are merely absent.
//!
//! `get_receipts_for_block` returns a bare `Vec`, so "this block has no receipts
//! stored" and "this block had no transactions" are the same value. Handing back
//! the empty list is a wrong answer rather than a reported failure, and once
//! history pruning lands (#6673) an absent receipt set becomes a normal
//! steady-state outcome rather than a corruption signal.

use ethrex_rpc::test_utils::{
    add_legacy_tx_blocks, call_http, default_context_with_storage, setup_store,
};

#[tokio::test]
async fn raw_receipts_reports_a_block_whose_receipts_are_absent() {
    let store = setup_store().await;
    // The harness stores blocks and their transactions but no receipts, which is
    // exactly the shape a pruned block presents: body present, receipts gone.
    add_legacy_tx_blocks(&store, 1, 1).await;
    let context = default_context_with_storage(store).await;

    let response = call_http(
        context,
        r#"{"jsonrpc":"2.0","method":"debug_getRawReceipts","params":["0x1"],"id":1}"#.to_string(),
    )
    .await;

    assert!(
        response.get("error").is_some(),
        "a block with 1 transaction and no stored receipts must report a failure \
         rather than returning an empty list; got {response}"
    );
    assert_ne!(
        response["result"].as_array().map(|a| a.len()),
        Some(0),
        "must not answer with an empty receipt list: {response}"
    );
}

#[tokio::test]
async fn raw_receipts_still_serves_genesis_as_empty() {
    let store = setup_store().await;
    add_legacy_tx_blocks(&store, 1, 1).await;
    let context = default_context_with_storage(store).await;

    // Genesis legitimately has no receipts and is short-circuited before the
    // completeness check, so it must keep answering with an empty list.
    let response = call_http(
        context,
        r#"{"jsonrpc":"2.0","method":"debug_getRawReceipts","params":["0x0"],"id":1}"#.to_string(),
    )
    .await;

    assert!(
        response.get("error").is_none(),
        "genesis must not be reported as a failure: {response}"
    );
    assert_eq!(
        response["result"].as_array().map(|a| a.len()),
        Some(0),
        "genesis has no receipts: {response}"
    );
}
