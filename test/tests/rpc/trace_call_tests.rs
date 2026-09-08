//! `debug_traceCall` coverage: parse-layer tests for parameter handling
//! ([`TraceCallRequest::parse`]) plus end-to-end tests over the full JSON-RPC HTTP
//! dispatch (`call_http`), which also guards the method's registration in
//! `map_debug_requests`.

use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};
use ethrex_rpc::tracing::TraceCallRequest;
use ethrex_rpc::utils::RpcErr;
use serde_json::{Value, json};

/// A minimal but complete `GenericTransaction` object for the first param.
fn call_object() -> Value {
    json!({
        "from": "0x1000000000000000000000000000000000000000",
        "to": "0xc000000000000000000000000000000000000000",
        "input": "0x",
        "maxFeePerBlobGas": null,
        "wrapperVersion": null,
    })
}

fn parse(params: Vec<Value>) -> Result<TraceCallRequest, RpcErr> {
    TraceCallRequest::parse(&Some(params))
}

/// Only the call object is required; block and traceConfig default in.
#[test]
fn trace_call_parses_with_only_call_object() {
    parse(vec![call_object()]).expect("call-object-only request must parse");
}

/// A `null` block param is treated as omitted (defaults to `latest`), matching
/// geth's acceptance of `traceCall(call, null, {...})`.
#[test]
fn trace_call_accepts_null_block_param() {
    parse(vec![
        call_object(),
        Value::Null,
        json!({ "tracer": "callTracer" }),
    ])
    .expect("null block param must be accepted");
}

/// A `null` traceConfig param is treated as omitted (defaults apply).
#[test]
fn trace_call_accepts_null_trace_config_param() {
    parse(vec![call_object(), json!("latest"), Value::Null])
        .expect("null traceConfig param must be accepted");
}

/// A valid `txIndex` in the traceConfig parses.
#[test]
fn trace_call_parses_tx_index() {
    parse(vec![
        call_object(),
        json!("latest"),
        json!({ "tracer": "callTracer", "txIndex": "0x2" }),
    ])
    .expect("txIndex request must parse");
}

/// Geth carries `stateOverrides` inside the 3rd config object. It must parse there.
#[test]
fn trace_call_parses_nested_state_overrides() {
    parse(vec![
        call_object(),
        json!("latest"),
        json!({ "stateOverrides": { "0x1000000000000000000000000000000000000000": { "balance": "0x1" } } }),
    ])
    .expect("nested stateOverrides must parse");
}

/// Same for `blockOverrides`.
#[test]
fn trace_call_parses_nested_block_overrides() {
    parse(vec![
        call_object(),
        json!("latest"),
        json!({ "blockOverrides": { "number": "0x1" } }),
    ])
    .expect("nested blockOverrides must parse");
}

/// An unknown key inside a Block Override Set is an error, not a silent drop.
#[test]
fn trace_call_rejects_unknown_block_override_field() {
    let result = parse(vec![
        call_object(),
        json!("latest"),
        json!({ "blockOverrides": { "number": "0x1", "notAField": "0x1" } }),
    ]);
    assert!(
        result.is_err(),
        "unknown blockOverrides field must be rejected"
    );
}

// ---------------------------------------------------------------------------
// End-to-end tests over the JSON-RPC HTTP dispatch.
// ---------------------------------------------------------------------------

/// Address we install synthetic bytecode at, via the State Override Set.
const CALLEE: &str = "0x000000000000000000000000000000000000cafe";
/// Caller. Funded through the override set so the test doesn't depend on
/// whatever the genesis fixture happens to allocate.
const CALLER: &str = "0x000000000000000000000000000000000000beef";

/// PUSH1 0x01, PUSH1 0x02, ADD, STOP — three executed opcodes plus the halt,
/// chosen so the expected struct-log sequence is unambiguous.
const ADD_THEN_STOP: &str = "0x600160020100";

fn trace_call_body(tracer: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            { "tracer": tracer },
            {
                CALLEE: { "code": ADD_THEN_STOP },
                CALLER: { "balance": "0xde0b6b3a7640000" }
            }
        ]
    })
    .to_string()
}

/// The opcodeTracer arm of `debug_traceCall` must return geth's structLogger
/// envelope, not panic. Guards the `todo!()` that previously sat in that arm.
#[tokio::test]
async fn trace_call_with_opcode_tracer_returns_struct_logs() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let response = call_http(context, trace_call_body("opcodeTracer")).await;

    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result, got: {response}"));

    // geth's structLogger envelope: {failed, gas, returnValue, structLogs}.
    assert_eq!(result["failed"], Value::Bool(false), "trace: {result}");
    assert!(result.get("gas").is_some(), "missing `gas`: {result}");
    assert!(
        result.get("returnValue").is_some(),
        "missing `returnValue`: {result}"
    );

    let logs = result["structLogs"]
        .as_array()
        .unwrap_or_else(|| panic!("structLogs missing or not an array: {result}"));
    let ops: Vec<&str> = logs.iter().filter_map(|l| l["op"].as_str()).collect();
    assert_eq!(
        ops,
        vec!["PUSH1", "PUSH1", "ADD", "STOP"],
        "unexpected opcode sequence: {result}"
    );
}

/// PUSH1 0x00, PUSH1 0x00, REVERT — reverts with empty return data.
const REVERT_EMPTY: &str = "0x60006000fd";

/// A reverting call must still produce a struct-log envelope with `failed: true`,
/// not an RPC error: a revert is a successful *execution* with a failed outcome.
///
/// Regression test, not a TDD cycle — `failed` is derived by `StructLoggerResult`,
/// a path already shared with `debug_traceTransaction`. It passed on first run.
#[tokio::test]
async fn trace_call_with_opcode_tracer_reports_revert_as_failed() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            { "tracer": "opcodeTracer" },
            {
                CALLEE: { "code": REVERT_EMPTY },
                CALLER: { "balance": "0xde0b6b3a7640000" }
            }
        ]
    })
    .to_string();

    let response = call_http(context, body).await;
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result, got: {response}"));

    assert_eq!(result["failed"], Value::Bool(true), "trace: {result}");
    let logs = result["structLogs"]
        .as_array()
        .unwrap_or_else(|| panic!("structLogs missing or not an array: {result}"));
    let ops: Vec<&str> = logs.iter().filter_map(|l| l["op"].as_str()).collect();
    assert_eq!(ops, vec!["PUSH1", "PUSH1", "REVERT"], "trace: {result}");
}

/// NUMBER, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN — returns the block
/// number as a 32-byte word. Lets a test observe the block context through
/// `returnValue` without depending on struct-log stack settings.
const RETURN_BLOCK_NUMBER: &str = "0x4360005260206000f3";

/// Geth's `debug_traceCall` takes `stateOverrides` as a field of the 3rd config
/// object (`TraceCallConfig` embeds `TraceConfig`), not as a 4th positional param.
/// A geth-shaped request must apply the overrides rather than silently drop them.
#[tokio::test]
async fn trace_call_applies_state_overrides_nested_in_config() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                "tracer": "opcodeTracer",
                "stateOverrides": {
                    CALLEE: { "code": ADD_THEN_STOP },
                    CALLER: { "balance": "0xde0b6b3a7640000" }
                }
            }
        ]
    })
    .to_string();

    let response = call_http(context, body).await;
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result, got: {response}"));

    let logs = result["structLogs"]
        .as_array()
        .unwrap_or_else(|| panic!("structLogs missing or not an array: {result}"));
    let ops: Vec<&str> = logs.iter().filter_map(|l| l["op"].as_str()).collect();
    assert_eq!(
        ops,
        vec!["PUSH1", "PUSH1", "ADD", "STOP"],
        "nested stateOverrides were dropped — no code ran at the callee: {result}"
    );
}

/// Same for `blockOverrides`: geth carries it inside the 3rd config object.
#[tokio::test]
async fn trace_call_applies_block_overrides_nested_in_config() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                "tracer": "opcodeTracer",
                "stateOverrides": {
                    CALLEE: { "code": RETURN_BLOCK_NUMBER },
                    CALLER: { "balance": "0xde0b6b3a7640000" }
                },
                "blockOverrides": { "number": "0x3039" }
            }
        ]
    })
    .to_string();

    let response = call_http(context, body).await;
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result, got: {response}"));

    let return_value = result["returnValue"]
        .as_str()
        .unwrap_or_else(|| panic!("returnValue missing or not a string: {result}"));
    assert!(
        return_value
            .trim_start_matches("0x")
            .trim_start_matches('0')
            == "3039",
        "NUMBER did not observe the overridden block number 0x3039: {result}"
    );
}

/// PUSH1 0x2a, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN — returns 42.
const RETURN_42: &str = "0x602a60005260206000f3";

/// `debug_traceCall` with the callTracer (ethrex's default) must emit geth's call
/// frame for the overridden callee. Coverage for the arm the branch's other tests
/// don't reach; `debug_traceCall` shipped with no callTracer test at all.
#[tokio::test]
async fn trace_call_with_call_tracer_returns_a_call_frame() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                "tracer": "callTracer",
                "stateOverrides": {
                    CALLEE: { "code": RETURN_42 },
                    CALLER: { "balance": "0xde0b6b3a7640000" }
                }
            }
        ]
    })
    .to_string();

    let response = call_http(context, body).await;
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result, got: {response}"));

    assert_eq!(result["type"], "CALL", "frame: {result}");
    assert_eq!(result["from"], CALLER, "frame: {result}");
    assert_eq!(result["to"], CALLEE, "frame: {result}");
    assert_eq!(result["error"], Value::Null, "frame: {result}");
    // The override's code ran: the frame's output is the 42 it returns.
    let output = result["output"]
        .as_str()
        .unwrap_or_else(|| panic!("output missing or not a string: {result}"));
    assert!(
        output.trim_start_matches("0x").trim_start_matches('0') == "2a",
        "call frame output does not show the overridden code's return: {result}"
    );
}

/// The prestateTracer must report the *overridden* pre-state, not the on-chain one:
/// the callee holds no code on chain, so the override's bytecode appearing here is
/// what proves the overlay is what the tracer reads through.
#[tokio::test]
async fn trace_call_with_prestate_tracer_reports_overridden_prestate() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                "tracer": "prestateTracer",
                "stateOverrides": {
                    CALLEE: { "code": RETURN_42 },
                    CALLER: { "balance": "0xde0b6b3a7640000" }
                }
            }
        ]
    })
    .to_string();

    let response = call_http(context, body).await;
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result, got: {response}"));

    let callee = result
        .get(CALLEE)
        .unwrap_or_else(|| panic!("callee missing from prestate: {result}"));
    assert_eq!(
        callee["code"], RETURN_42,
        "prestate reports on-chain code instead of the override: {result}"
    );
    let caller = result
        .get(CALLER)
        .unwrap_or_else(|| panic!("caller missing from prestate: {result}"));
    assert_eq!(
        caller["balance"], "0xde0b6b3a7640000",
        "prestate reports the on-chain balance instead of the override: {result}"
    );
}

/// A State Override Set can only be applied when the call runs on top of a block whose
/// post-state is stored. With `txIndex` the blockchain layer rebuilds the parent state
/// and re-runs the block's own transactions, and an overlay installed for that would be
/// visible to them — so the request is refused rather than answered with a trace
/// computed against inputs the block never saw.
#[tokio::test]
async fn trace_call_rejects_state_overrides_combined_with_tx_index() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                "tracer": "opcodeTracer",
                "txIndex": "0x0",
                "stateOverrides": { CALLEE: { "code": ADD_THEN_STOP } }
            }
        ]
    })
    .to_string();

    let response = call_http(context, body).await;
    let error = response
        .get("error")
        .unwrap_or_else(|| panic!("expected an error, got: {response}"));
    let message = error["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("stateOverrides"),
        "error should explain the stateOverrides limitation, got: {response}"
    );
    // `txIndex` is right there in the params, so this is a permanent client error that
    // retrying never fixes. It must not come back as -32603 Internal error, which tells a
    // caller the node faulted. -32000 is what `RpcErr::BadParams` maps to throughout this
    // crate; the JSON-RPC standard code for this would be -32602, but that mapping is
    // repo-wide and not this PR's to change.
    assert_eq!(
        error["code"], -32000,
        "a request-shape error must not be reported as an internal error: {response}"
    );
}
