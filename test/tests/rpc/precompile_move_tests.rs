//! `movePrecompileToAddress` (geth State Override Set) coverage.
//!
//! Uses the identity precompile at 0x04, which echoes its calldata — a relocation
//! either dispatches it (output == input) or it doesn't (output empty), with no
//! ambiguous middle ground.

use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};
use serde_json::json;

/// Identity / datacopy precompile.
const IDENTITY: &str = "0x0000000000000000000000000000000000000004";
/// Where we relocate it to.
const RELOCATED: &str = "0x0000000000000000000000000000000000000aaa";
const CALLER: &str = "0x000000000000000000000000000000000000beef";
/// Calldata echoed back by the identity precompile.
const PAYLOAD: &str = "0x11223344";

fn eth_call(to: &str, overrides: serde_json::Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_call",
        "params": [
            { "from": CALLER, "to": to, "data": PAYLOAD },
            "latest",
            overrides
        ]
    })
    .to_string()
}

/// After `movePrecompileToAddress`, the destination must execute the precompile.
#[tokio::test]
async fn moved_precompile_executes_at_destination() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = eth_call(
        RELOCATED,
        json!({ IDENTITY: { "movePrecompileToAddress": RELOCATED } }),
    );
    let response = call_http(context, body).await;

    let result = response
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or_else(|| panic!("expected a string result, got: {response}"));

    assert!(
        result.contains("11223344"),
        "identity precompile did not run at the relocated address; got: {result}"
    );
}

/// The vacated address must stop behaving as a precompile: geth turns it into a
/// regular (here, empty) account, so the call succeeds returning no data.
#[tokio::test]
async fn vacated_precompile_address_is_a_normal_account() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = eth_call(
        IDENTITY,
        json!({ IDENTITY: { "movePrecompileToAddress": RELOCATED } }),
    );
    let response = call_http(context, body).await;

    let result = response
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or_else(|| panic!("expected a string result, got: {response}"));

    assert!(
        !result.contains("11223344"),
        "vacated address still ran the identity precompile; got: {result}"
    );
}
