//! Methods that differential testing on glamsterdam-devnet-8 found missing.
//!
//! Each of these returned `Method not found: …` from ethrex while geth served
//! them, so tooling that probes the standard surface reported ethrex as lacking
//! functionality it largely already had internally.

use ethrex_rpc::test_utils::{
    add_eip1559_tx_blocks, add_legacy_tx_blocks, call_http, default_context_with_storage,
    setup_store,
};
use serde_json::Value;

use ethrex_rpc::test_utils::TestContext;

async fn context_with_one_block() -> TestContext {
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
        &context,
        call("eth_getUncleCountByBlockNumber", r#"["0x1"]"#),
    )
    .await;
    assert_eq!(
        by_number["result"], "0x0",
        "a post-merge block has no ommers; got {by_number}"
    );

    let block_hash = call_http(
        &context,
        call("eth_getBlockByNumber", r#"["0x1",false]"#),
    )
    .await["result"]["hash"]
        .as_str()
        .expect("block should have a hash")
        .to_owned();
    let by_hash = call_http(
        &context,
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
        &context,
        call("eth_getUncleCountByBlockNumber", r#"["0x999999"]"#),
    )
    .await;
    assert_eq!(unknown["result"], Value::Null, "got {unknown}");
}

#[tokio::test]
async fn new_block_filter_registers_and_polls() {
    let context = context_with_one_block().await;

    let created = call_http(&context, call("eth_newBlockFilter", "[]")).await;
    let id = created["result"]
        .as_str()
        .unwrap_or_else(|| panic!("eth_newBlockFilter must return a filter id: {created}"))
        .to_owned();
    assert!(id.starts_with("0x"), "filter id must be hex: {id}");

    // The filter anchors at the head at registration, so an immediate poll
    // reports nothing rather than replaying history.
    let changes = call_http(
        &context,
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

    let block = call_http(&context, call("eth_getBlockByNumber", r#"["0x1",true]"#)).await;
    let tx_hash = block["result"]["transactions"][0]["hash"]
        .as_str()
        .expect("block should contain a transaction")
        .to_owned();
    let block_hash = block["result"]["hash"].as_str().expect("hash").to_owned();

    let by_hash = call_http(
        &context,
        call("eth_getRawTransactionByHash", &format!(r#"["{tx_hash}"]"#)),
    )
    .await;
    let raw = by_hash["result"]
        .as_str()
        .unwrap_or_else(|| panic!("eth_getRawTransactionByHash must return RLP: {by_hash}"));
    assert!(raw.starts_with("0x") && raw.len() > 2, "got {raw}");

    // The `debug_` spelling already existed; the `eth_` one must agree with it.
    let debug_form = call_http(
        &context,
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
        let response = call_http(&context, call(method, &params)).await;
        assert_eq!(
            response["result"], by_hash["result"],
            "{method} must return the same bytes as by-hash; got {response}"
        );
    }

    // An index past the end is `null`, not an error.
    let past_end = call_http(
        &context,
        call(
            "eth_getRawTransactionByBlockNumberAndIndex",
            r#"["0x1","0x9"]"#,
        ),
    )
    .await;
    assert_eq!(past_end["result"], Value::Null, "got {past_end}");
}

/// Typed transactions come back as the EIP-2718 envelope (`type || payload`),
/// the encoding whose keccak is the transaction hash, from every raw getter.
/// Legacy transactions encode the same either way, so this needs a typed one.
#[tokio::test]
async fn raw_transaction_getters_return_the_eip2718_envelope() {
    let store = setup_store().await;
    add_eip1559_tx_blocks(&store, 1, 1).await;
    let context = default_context_with_storage(store).await;

    let block = call_http(&context, call("eth_getBlockByNumber", r#"["0x1",false]"#)).await;
    let tx_hash = block["result"]["transactions"][0]
        .as_str()
        .unwrap_or_else(|| panic!("block should contain a transaction: {block}"))
        .to_owned();
    let block_hash = block["result"]["hash"].as_str().expect("hash").to_owned();

    for (method, params) in [
        ("eth_getRawTransactionByHash", format!(r#"["{tx_hash}"]"#)),
        ("debug_getRawTransaction", format!(r#"["{tx_hash}"]"#)),
        (
            "eth_getRawTransactionByBlockNumberAndIndex",
            r#"["0x1","0x0"]"#.to_owned(),
        ),
        (
            "eth_getRawTransactionByBlockHashAndIndex",
            format!(r#"["{block_hash}","0x0"]"#),
        ),
    ] {
        let response = call_http(&context, call(method, &params)).await;
        let raw = response["result"]
            .as_str()
            .unwrap_or_else(|| panic!("{method} must return bytes: {response}"));
        let bytes = hex::decode(raw.trim_start_matches("0x")).expect("hex");
        assert_eq!(
            bytes.first(),
            Some(&0x02),
            "{method} must start with the EIP-1559 type byte, got {raw}"
        );
        let hash = format!(
            "0x{}",
            hex::encode(ethrex_crypto::keccak::keccak_hash(&bytes))
        );
        assert_eq!(
            hash, tx_hash,
            "{method}: the keccak of the returned bytes must be the transaction hash"
        );
    }
}
