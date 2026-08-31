//! Methods that differential testing on glamsterdam-devnet-8 found missing.
//!
//! Each of these returned `Method not found: …` from ethrex while geth served
//! them, so tooling that probes the standard surface reported ethrex as lacking
//! functionality it largely already had internally.

use ethrex_rpc::test_utils::{
    add_legacy_tx_blocks, call_http, default_context_with_storage, setup_store,
};
use serde_json::Value;

async fn context_with_one_block() -> ethrex_rpc::RpcApiContext {
    let store = setup_store().await;
    add_legacy_tx_blocks(&store, 1, 1).await;
    default_context_with_storage(store).await
}

fn call(method: &str, params: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","method":"{method}","params":{params},"id":1}}"#)
}

#[tokio::test]
async fn uncle_count_is_zero_for_a_known_block_and_null_for_an_unknown_one() {
    let context = context_with_one_block().await;

    let by_number = call_http(
        context.clone(),
        call("eth_getUncleCountByBlockNumber", r#"["0x1"]"#),
    )
    .await;
    assert_eq!(
        by_number["result"], "0x0",
        "a post-merge block has no ommers; got {by_number}"
    );

    let block_hash = call_http(
        context.clone(),
        call("eth_getBlockByNumber", r#"["0x1",false]"#),
    )
    .await["result"]["hash"]
        .as_str()
        .expect("block should have a hash")
        .to_owned();
    let by_hash = call_http(
        context.clone(),
        call(
            "eth_getUncleCountByBlockHash",
            &format!(r#"["{block_hash}"]"#),
        ),
    )
    .await;
    assert_eq!(by_hash["result"], "0x0", "got {by_hash}");

    // An unknown block must be `null`, not an error — matching the
    // transaction-count getters this mirrors.
    let unknown = call_http(
        context,
        call("eth_getUncleCountByBlockNumber", r#"["0x999999"]"#),
    )
    .await;
    assert_eq!(unknown["result"], Value::Null, "got {unknown}");
}

#[tokio::test]
async fn new_block_filter_registers_and_polls() {
    let context = context_with_one_block().await;

    let created = call_http(context.clone(), call("eth_newBlockFilter", "[]")).await;
    let id = created["result"]
        .as_str()
        .unwrap_or_else(|| panic!("eth_newBlockFilter must return a filter id: {created}"))
        .to_owned();
    assert!(id.starts_with("0x"), "filter id must be hex: {id}");

    // The filter anchors at the head at registration, so an immediate poll
    // reports nothing rather than replaying history.
    let changes = call_http(
        context,
        call("eth_getFilterChanges", &format!(r#"["{id}"]"#)),
    )
    .await;
    let hashes = changes["result"]
        .as_array()
        .unwrap_or_else(|| panic!("getFilterChanges must return an array: {changes}"));
    assert!(
        hashes.is_empty(),
        "a freshly registered block filter has no new blocks yet, got {hashes:?}"
    );
}

#[tokio::test]
async fn raw_transaction_getters_agree_across_all_three_spellings() {
    let context = context_with_one_block().await;

    let block = call_http(
        context.clone(),
        call("eth_getBlockByNumber", r#"["0x1",true]"#),
    )
    .await;
    let tx_hash = block["result"]["transactions"][0]["hash"]
        .as_str()
        .expect("block should contain a transaction")
        .to_owned();
    let block_hash = block["result"]["hash"].as_str().expect("hash").to_owned();

    let by_hash = call_http(
        context.clone(),
        call("eth_getRawTransactionByHash", &format!(r#"["{tx_hash}"]"#)),
    )
    .await;
    let raw = by_hash["result"]
        .as_str()
        .unwrap_or_else(|| panic!("eth_getRawTransactionByHash must return RLP: {by_hash}"));
    assert!(raw.starts_with("0x") && raw.len() > 2, "got {raw}");

    // The `debug_` spelling already existed; the `eth_` one must agree with it.
    let debug_form = call_http(
        context.clone(),
        call("debug_getRawTransaction", &format!(r#"["{tx_hash}"]"#)),
    )
    .await;
    assert_eq!(
        by_hash["result"], debug_form["result"],
        "eth_ and debug_ spellings must return identical bytes"
    );

    for (method, params) in [
        (
            "eth_getRawTransactionByBlockNumberAndIndex",
            r#"["0x1","0x0"]"#.to_owned(),
        ),
        (
            "eth_getRawTransactionByBlockHashAndIndex",
            format!(r#"["{block_hash}","0x0"]"#),
        ),
    ] {
        let response = call_http(context.clone(), call(method, &params)).await;
        assert_eq!(
            response["result"], by_hash["result"],
            "{method} must return the same bytes as by-hash; got {response}"
        );
    }

    // An index past the end is `null`, not an error.
    let past_end = call_http(
        context,
        call(
            "eth_getRawTransactionByBlockNumberAndIndex",
            r#"["0x1","0x9"]"#,
        ),
    )
    .await;
    assert_eq!(past_end["result"], Value::Null, "got {past_end}");
}
