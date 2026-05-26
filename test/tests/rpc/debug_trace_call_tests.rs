use ethrex_common::Address;
use serde_json::{Value, json};

use super::helpers::{
    rpc_call, rpc_call_expect_err, setup_genesis_only, setup_single_transfer_block,
};

fn transfer_call(sender: Address, recipient: Address) -> Value {
    transfer_call_with_nonce(sender, recipient, 0)
}

/// Like [`transfer_call`] but pins the call's `nonce` to match what the
/// account already has in state. ethrex's VM enforces nonce equality even on
/// simulated calls (same behaviour as `eth_call`), so post-block tests must
/// either pre-fund the sender fresh or pass an explicit nonce.
fn transfer_call_with_nonce(sender: Address, recipient: Address, nonce: u64) -> Value {
    json!({
        "from": format!("{sender:#x}"),
        "to": format!("{recipient:#x}"),
        "value": "0xde0b6b3a7640000",
        "nonce": format!("{nonce:#x}"),
    })
}

#[tokio::test]
async fn trace_call_default_call_tracer_at_latest() {
    let env = setup_genesis_only().await;
    let recipient = Address::from_low_u64_be(0xBB);

    let result = rpc_call(
        &env.store,
        "debug_traceCall",
        vec![transfer_call(env.sender, recipient), json!("latest")],
    )
    .await;

    let obj = result.as_object().expect("response should be an object");
    assert_eq!(obj["type"].as_str().unwrap(), "CALL");
    assert_eq!(
        obj["from"].as_str().unwrap().to_lowercase(),
        format!("{:#x}", env.sender).to_lowercase()
    );
    assert_eq!(
        obj["to"].as_str().unwrap().to_lowercase(),
        format!("{recipient:#x}").to_lowercase()
    );
    assert!(obj.contains_key("gas"), "missing 'gas'");
}

#[tokio::test]
async fn trace_call_defaults_block_to_latest() {
    let env = setup_genesis_only().await;
    let recipient = Address::from_low_u64_be(0xBB);

    // Omitting the second parameter should be equivalent to passing "latest".
    let result = rpc_call(
        &env.store,
        "debug_traceCall",
        vec![transfer_call(env.sender, recipient)],
    )
    .await;
    let obj = result.as_object().expect("response should be an object");
    assert_eq!(obj["type"].as_str().unwrap(), "CALL");
}

#[tokio::test]
async fn trace_call_by_block_number() {
    // Trace against state *after* a tx was applied — sender nonce is 1.
    let env = setup_single_transfer_block().await;
    let recipient = Address::from_low_u64_be(0xBB);

    let result = rpc_call(
        &env.store,
        "debug_traceCall",
        vec![
            transfer_call_with_nonce(env.sender, recipient, 1),
            json!(format!("{:#x}", env.block.header.number)),
        ],
    )
    .await;
    let obj = result.as_object().expect("response should be an object");
    assert_eq!(obj["type"].as_str().unwrap(), "CALL");
}

#[tokio::test]
async fn trace_call_by_block_hash_eip1898() {
    let env = setup_single_transfer_block().await;
    let recipient = Address::from_low_u64_be(0xBB);

    let result = rpc_call(
        &env.store,
        "debug_traceCall",
        vec![
            transfer_call_with_nonce(env.sender, recipient, 1),
            json!({ "blockHash": format!("{:#x}", env.block.hash()) }),
        ],
    )
    .await;
    let obj = result.as_object().expect("response should be an object");
    assert_eq!(obj["type"].as_str().unwrap(), "CALL");
}

#[tokio::test]
async fn trace_call_prestate_tracer() {
    let env = setup_genesis_only().await;
    let recipient = Address::from_low_u64_be(0xBB);

    let result = rpc_call(
        &env.store,
        "debug_traceCall",
        vec![
            transfer_call(env.sender, recipient),
            json!("latest"),
            json!({"tracer": "prestateTracer"}),
        ],
    )
    .await;
    // prestateTracer returns an object keyed by address with {balance, nonce, ...}.
    let obj = result.as_object().expect("response should be an object");
    let sender_key = format!("{:#x}", env.sender).to_lowercase();
    let entry = obj
        .iter()
        .find(|(k, _)| k.to_lowercase() == sender_key)
        .map(|(_, v)| v)
        .expect("sender must appear in prestate");
    assert!(entry["balance"].is_string());
}

#[tokio::test]
async fn trace_call_opcode_tracer() {
    let env = setup_genesis_only().await;
    let recipient = Address::from_low_u64_be(0xBB);

    let result = rpc_call(
        &env.store,
        "debug_traceCall",
        vec![
            transfer_call(env.sender, recipient),
            json!("latest"),
            json!({"tracer": "opcodeTracer"}),
        ],
    )
    .await;
    // opcodeTracer emits the structLogger wrapper: {failed, gas, returnValue, structLogs}.
    let obj = result.as_object().expect("response should be an object");
    assert!(obj.contains_key("failed"));
    assert!(obj.contains_key("gas"));
    assert!(obj.contains_key("structLogs"));
}

#[tokio::test]
async fn trace_call_unknown_block_errors() {
    let env = setup_genesis_only().await;
    let recipient = Address::from_low_u64_be(0xBB);

    let err = rpc_call_expect_err(
        &env.store,
        "debug_traceCall",
        vec![transfer_call(env.sender, recipient), json!("0x10000")],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Block not found"),
        "expected block-not-found error, got: {msg}"
    );
}

#[tokio::test]
async fn trace_call_empty_params_errors() {
    let env = setup_genesis_only().await;
    let err = rpc_call_expect_err(&env.store, "debug_traceCall", vec![]).await;
    let msg = format!("{err:?}");
    assert!(msg.contains("params"), "expected BadParams, got: {msg}");
}
