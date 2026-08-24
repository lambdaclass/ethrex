//! `eth_feeHistory` must not abort when the requested window sits below the
//! earliest block the node still holds.
//!
//! `get_range` clamps its bounds independently — the finish by the latest block,
//! the start up to the earliest retained block — so on a snap-synced node
//! (earliest = pivot) a window entirely below the pivot produces start > end.
//! The block count is then computed as `end - start + 1` on u64, which wraps in
//! release and turns into a `vec![0; ~1.8e19]` allocation abort: a one-request
//! process kill from an unauthenticated RPC.

use ethrex_rpc::test_utils::{
    add_legacy_tx_blocks, call_http, default_context_with_storage, setup_store,
};

#[tokio::test]
async fn fee_history_below_the_earliest_retained_block_is_empty_not_a_crash() {
    let store = setup_store().await;
    add_legacy_tx_blocks(&store, 6, 1).await;
    // Simulate a snap-synced datadir: history below the pivot is not retained.
    store.update_earliest_block_number(5).await.unwrap();
    let context = default_context_with_storage(store).await;

    // newestBlock (0x1) is below earliest (5), so the whole window is unavailable.
    let response = call_http(
        context,
        r#"{"jsonrpc":"2.0","method":"eth_feeHistory","params":["0x1","0x1",[]],"id":1}"#
            .to_string(),
    )
    .await;

    assert!(
        response.get("error").is_none(),
        "a window below retained history must not error: {response}"
    );
    let result = &response["result"];
    assert_eq!(
        result["baseFeePerGas"].as_array().map(|a| a.len()),
        Some(0),
        "expected an empty fee history, got {response}"
    );
    assert_eq!(
        result["gasUsedRatio"].as_array().map(|a| a.len()),
        Some(0),
        "expected an empty fee history, got {response}"
    );
}

#[tokio::test]
async fn fee_history_partially_below_earliest_still_serves_the_retained_part() {
    let store = setup_store().await;
    add_legacy_tx_blocks(&store, 6, 1).await;
    store.update_earliest_block_number(4).await.unwrap();
    let context = default_context_with_storage(store).await;

    // Ask for 10 blocks ending at 6; only 4..=6 are retained.
    let response = call_http(
        context,
        r#"{"jsonrpc":"2.0","method":"eth_feeHistory","params":["0xa","0x6",[]],"id":1}"#
            .to_string(),
    )
    .await;

    assert!(
        response.get("error").is_none(),
        "a partially-available window must still be served: {response}"
    );
    assert_eq!(
        response["result"]["oldestBlock"], "0x4",
        "oldestBlock must be clamped to the earliest retained block, got {response}"
    );
}
