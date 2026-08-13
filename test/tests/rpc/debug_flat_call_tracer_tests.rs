use ethrex_common::H256;
use serde_json::{Value, json};

use super::helpers::{
    rpc_call, rpc_call_expect_err, setup_single_deploy_block, setup_single_transfer_block,
};

#[tokio::test]
async fn trace_tx_flat_call_tracer_value_transfer() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", env.tx_hash)),
            json!({"tracer": "flatCallTracer"}),
        ],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    assert_eq!(
        arr.len(),
        1,
        "value transfer should produce a single flat frame"
    );

    let f = &arr[0];
    assert_eq!(f["type"], "call");
    assert_eq!(f["traceAddress"], json!([]));
    assert_eq!(f["subtraces"], 0);
    assert_eq!(f["action"]["callType"], "call");
    assert!(f["action"]["from"].is_string());
    assert!(f["action"]["to"].is_string());
    assert!(f["action"]["gas"].is_string());
    // Call results carry gasUsed + output, NOT address/code.
    assert!(f["result"]["gasUsed"].is_string());
    assert!(f["result"]["output"].is_string());
    assert!(f["result"].get("address").is_none());
}

#[tokio::test]
async fn trace_tx_flat_call_tracer_create() {
    let env = setup_single_deploy_block().await;

    let result = rpc_call(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", env.tx_hash)),
            json!({"tracer": "flatCallTracer"}),
        ],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    assert!(
        !arr.is_empty(),
        "deploy tx should produce at least one frame"
    );
    // The top frame for a contract deployment is the CREATE.
    let f = &arr[0];
    assert_eq!(f["type"], "create");
    assert_eq!(f["action"]["creationMethod"], "create");
    // Init code lives under `init`, not `input`.
    assert!(
        f["action"]["init"].is_string(),
        "create action must carry `init`"
    );
    assert!(
        f["action"].get("input").is_none(),
        "create action must not carry `input`"
    );
    assert!(
        f["action"].get("to").is_none(),
        "create action must not carry `to`"
    );
    // Result carries the deployed `address` and runtime `code`.
    assert!(
        f["result"]["address"].is_string(),
        "create result must carry deployed `address`"
    );
    assert!(
        f["result"]["code"].is_string(),
        "create result must carry deployed `code`"
    );
    assert!(
        f["result"].get("output").is_none(),
        "create result must not carry `output`"
    );
}

#[tokio::test]
async fn trace_tx_flat_call_tracer_unknown_hash_errors() {
    let env = setup_single_transfer_block().await;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", H256::from_low_u64_be(0xdeadbeef))),
            json!({"tracer": "flatCallTracer"}),
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
async fn trace_block_flat_call_tracer() {
    let env = setup_single_transfer_block().await;

    let result: Value = rpc_call(
        &env.store,
        "debug_traceBlockByNumber",
        vec![
            json!(format!("{:#x}", env.block.header.number)),
            json!({"tracer": "flatCallTracer"}),
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
    let frames = entry["result"]
        .as_array()
        .expect("result should be a flat frame array");
    assert_eq!(frames.len(), 1, "value transfer is a single frame");
    assert_eq!(frames[0]["type"], "call");
    assert_eq!(frames[0]["action"]["callType"], "call");
}
