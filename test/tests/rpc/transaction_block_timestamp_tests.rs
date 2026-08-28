//! Transaction objects must carry `blockTimestamp` once mined.
//!
//! Differential testing on glamsterdam-devnet-8 found ethrex's transaction
//! objects were the only ones without it: geth returned 21 fields including
//! `blockTimestamp`, ethrex 20 without. Receipts and logs already carried it;
//! transactions were missed. Consumers use it to date a transaction without a
//! second round trip for the block.

use ethrex_rpc::test_utils::{
    add_legacy_tx_blocks, call_http, default_context_with_storage, setup_store,
};

/// `test_header` stamps every test block with this timestamp.
const EXPECTED: &str = "0x3e8";

async fn block_with_one_tx() -> serde_json::Value {
    let store = setup_store().await;
    add_legacy_tx_blocks(&store, 1, 1).await;
    let context = default_context_with_storage(store).await;
    let body = r#"{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["0x1",true],"id":1}"#;
    call_http(context, body.to_string()).await
}

#[tokio::test]
async fn full_block_transactions_carry_block_timestamp() {
    let response = block_with_one_tx().await;
    let tx = &response["result"]["transactions"][0];
    assert_eq!(
        tx["blockTimestamp"], EXPECTED,
        "a mined transaction must expose blockTimestamp; got {response}"
    );
}

#[tokio::test]
async fn get_transaction_by_hash_carries_block_timestamp() {
    let store = setup_store().await;
    add_legacy_tx_blocks(&store, 1, 1).await;
    let context = default_context_with_storage(store).await;

    let block = call_http(
        context.clone(),
        r#"{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["0x1",true],"id":1}"#
            .to_string(),
    )
    .await;
    let hash = block["result"]["transactions"][0]["hash"]
        .as_str()
        .unwrap_or_else(|| panic!("block should contain a transaction: {block}"))
        .to_owned();

    let response = call_http(
        context,
        format!(
            r#"{{"jsonrpc":"2.0","method":"eth_getTransactionByHash","params":["{hash}"],"id":1}}"#
        ),
    )
    .await;
    assert_eq!(
        response["result"]["blockTimestamp"], EXPECTED,
        "eth_getTransactionByHash must expose blockTimestamp; got {response}"
    );
}

#[tokio::test]
async fn get_transaction_by_block_and_index_carries_block_timestamp() {
    let store = setup_store().await;
    add_legacy_tx_blocks(&store, 1, 1).await;
    let context = default_context_with_storage(store.clone()).await;

    let by_number = call_http(
        context.clone(),
        r#"{"jsonrpc":"2.0","method":"eth_getTransactionByBlockNumberAndIndex","params":["0x1","0x0"],"id":1}"#.to_string(),
    )
    .await;
    assert_eq!(
        by_number["result"]["blockTimestamp"], EXPECTED,
        "eth_getTransactionByBlockNumberAndIndex must expose blockTimestamp; got {by_number}"
    );

    // The by-hash-and-index handler reads the body but previously never loaded
    // the header, so it is the likeliest of the three to regress.
    let block_hash = call_http(
        context.clone(),
        r#"{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["0x1",false],"id":1}"#
            .to_string(),
    )
    .await["result"]["hash"]
        .as_str()
        .expect("block should have a hash")
        .to_owned();
    let by_hash = call_http(
        context,
        format!(
            r#"{{"jsonrpc":"2.0","method":"eth_getTransactionByBlockHashAndIndex","params":["{block_hash}","0x0"],"id":1}}"#
        ),
    )
    .await;
    assert_eq!(
        by_hash["result"]["blockTimestamp"], EXPECTED,
        "eth_getTransactionByBlockHashAndIndex must expose blockTimestamp; got {by_hash}"
    );
}
