//! End-to-end coverage for geth's State Override Set on `eth_call` and
//! `eth_estimateGas`.
//!
//! The branch's other override coverage is unit-level (parse tests plus the
//! `OverlaidVmDatabase` tests). These go through the full JSON-RPC HTTP dispatch so
//! the whole chain is exercised: parse -> `into_overrides` -> `new_overlaid_evm` ->
//! LEVM. Each asserts on an *executed* result rather than on the parsed request, so
//! an override that parses but never reaches the EVM fails the test.

use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};
use serde_json::{Value, json};

/// Address the overrides install synthetic bytecode at.
const CALLEE: &str = "0x000000000000000000000000000000000000cafe";
/// Caller, funded through the override set so the tests don't depend on whatever
/// the genesis fixture happens to allocate.
const CALLER: &str = "0x000000000000000000000000000000000000beef";
const ONE_ETH: &str = "0xde0b6b3a7640000";

/// PUSH1 0x2a, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN — returns 42.
const RETURN_42: &str = "0x602a60005260206000f3";
/// SELFBALANCE, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN.
const RETURN_SELF_BALANCE: &str = "0x4760005260206000f3";
/// PUSH1 0x00, SLOAD, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN.
const RETURN_SLOT_ZERO: &str = "0x60005460005260206000f3";
/// PUSH1 0x01, PUSH1 0x00, SSTORE, STOP — a cold zero->nonzero SSTORE, so far more
/// than the 21000 a plain value transfer costs.
const STORE_ONE: &str = "0x600160005500";

const SLOT_ZERO: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

fn request(method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string()
}

/// The `result` string, or a panic naming the whole response (which carries the
/// JSON-RPC `error` object when a request was rejected).
fn expect_result(response: &Value) -> &str {
    response
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or_else(|| panic!("expected a string result, got: {response}"))
}

/// Strip the `0x` and leading zeros so a 32-byte word can be compared to the small
/// value it encodes.
fn word_value(result: &str) -> &str {
    let trimmed = result.trim_start_matches("0x").trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

/// A `code` override must make `eth_call` execute that code at an address which
/// holds none on chain.
#[tokio::test]
async fn call_executes_overridden_code() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                CALLEE: { "code": RETURN_42 },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );

    let response = call_http(context, body).await;
    assert_eq!(word_value(expect_result(&response)), "2a");
}

/// Without the override the same call runs against an empty account and returns no
/// data. Control for the test above: proves the assertion tracks the override rather
/// than something the genesis fixture already provides.
#[tokio::test]
async fn call_without_overrides_hits_the_empty_account() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([{ "from": CALLER, "to": CALLEE, "data": "0x" }, "latest"]),
    );

    let response = call_http(context, body).await;
    assert_eq!(expect_result(&response), "0x");
}

/// A `balance` override must be observable from inside the EVM, not just at the
/// accounting layer: SELFBALANCE reads it through `get_account_state`.
#[tokio::test]
async fn call_observes_overridden_balance() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                CALLEE: { "code": RETURN_SELF_BALANCE, "balance": "0x1234" },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );

    let response = call_http(context, body).await;
    assert_eq!(word_value(expect_result(&response)), "1234");
}

/// A `state` override must be visible to SLOAD.
#[tokio::test]
async fn call_observes_overridden_storage() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                CALLEE: { "code": RETURN_SLOT_ZERO, "state": { SLOT_ZERO: "0x5678" } },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );

    let response = call_http(context, body).await;
    assert_eq!(word_value(expect_result(&response)), "5678");
}

/// `eth_estimateGas` short-circuits a plain value transfer to 21000 without
/// executing anything. That shortcut is wrong when a `code` override puts code at
/// the destination, so the handler skips it whenever overrides are present — this
/// guards that skip.
#[tokio::test]
async fn estimate_gas_with_code_override_skips_the_value_transfer_short_circuit() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    // Control: no overrides, empty destination -> the 21000 short-circuit.
    let plain = request(
        "eth_estimateGas",
        json!([{ "from": CALLER, "to": CALLEE, "value": "0x0" }, "latest"]),
    );
    let response = call_http(context.clone(), plain).await;
    assert_eq!(
        expect_result(&response),
        "0x5208",
        "expected the plain-transfer short-circuit (21000)"
    );

    // Same call, but the destination now holds a cold SSTORE.
    let overridden = request(
        "eth_estimateGas",
        json!([
            { "from": CALLER, "to": CALLEE, "value": "0x0" },
            "latest",
            {
                CALLEE: { "code": STORE_ONE },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );
    let response = call_http(context, overridden).await;
    let estimate = expect_result(&response);
    let gas = u64::from_str_radix(estimate.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("estimate {estimate} is not hex: {e}"));
    assert!(
        gas > 21_000,
        "short-circuit was not skipped: estimate {estimate} does not cover the SSTORE"
    );
}
