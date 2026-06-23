use ethrex_common::H256;
use serde_json::{Value, json};

use super::helpers::{rpc_call, rpc_call_expect_err, setup_single_transfer_block};

#[tokio::test]
async fn trace_tx_four_byte_tracer_value_transfer_is_empty() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", env.tx_hash)),
            json!({"tracer": "4byteTracer"}),
        ],
    )
    .await;

    // A simple ETH transfer has no nested calls and the tracer skips the
    // top-level call, so the result must be the empty map.
    let obj = result.as_object().expect("response should be an object");
    assert!(obj.is_empty(), "simple transfer has no 4-byte selectors");
}

#[tokio::test]
async fn trace_tx_four_byte_tracer_unknown_hash_errors() {
    let env = setup_single_transfer_block().await;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", H256::from_low_u64_be(0xdeadbeef))),
            json!({"tracer": "4byteTracer"}),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Transaction not Found"),
        "expected tx-not-found error, got: {msg}"
    );
}

#[tokio::test]
async fn trace_block_four_byte_tracer() {
    let env = setup_single_transfer_block().await;

    let result: Value = rpc_call(
        &env.store,
        "debug_traceBlockByNumber",
        vec![
            json!(format!("{:#x}", env.block.header.number)),
            json!({"tracer": "4byteTracer"}),
        ],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    assert_eq!(arr.len(), 1, "one tx in block");
    let entry = arr[0].as_object().expect("entry should be an object");
    assert_eq!(
        entry["txHash"].as_str().unwrap().to_lowercase(),
        format!("{:#x}", env.tx_hash).to_lowercase()
    );
    let selectors = entry["result"]
        .as_object()
        .expect("result should be a selector map");
    assert!(selectors.is_empty(), "value transfer yields no selectors");
}
