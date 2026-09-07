//! `movePrecompileToAddress` (geth State Override Set) coverage, over `eth_call` and
//! `debug_traceCall`.
//!
//! Uses the identity precompile at 0x04, which echoes its calldata — a relocation
//! either dispatches it (output == input) or it doesn't (output empty), with no
//! ambiguous middle ground.
//!
//! `debug_traceCall` reaches the EVM by a different route than `eth_call`: it is built
//! inside `Blockchain::build_call_trace_vm` rather than at the RPC layer, so the
//! relocations have to be installed there too and the `eth_call` tests above say nothing
//! about it. The tracer used is the callTracer, whose frame reports the call's `output`
//! directly.

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

fn trace_call(to: &str, overrides: serde_json::Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            { "from": CALLER, "to": to, "data": PAYLOAD },
            "latest",
            { "tracer": "callTracer", "stateOverrides": overrides }
        ]
    })
    .to_string()
}

/// The call frame's return data, as the frame reports it.
///
/// `CallTraceFrame::output` carries `skip_serializing_if = "Bytes::is_empty"` to match
/// geth's `output,omitempty`, so an absent key *is* the empty-return-data answer rather
/// than a malformed frame — which is precisely the signal the vacated-address test wants.
fn frame_output(response: &serde_json::Value) -> String {
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result, got: {response}"));
    assert_eq!(
        result["error"],
        serde_json::Value::Null,
        "traced call failed: {result}"
    );
    match result.get("output") {
        Some(output) => output
            .as_str()
            .unwrap_or_else(|| panic!("`output` is not a string: {result}"))
            .to_owned(),
        None => String::from("0x"),
    }
}

/// `debug_traceCall` must dispatch the relocated precompile too, not just `eth_call`.
#[tokio::test]
async fn trace_call_dispatches_a_moved_precompile_at_the_destination() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = trace_call(
        RELOCATED,
        json!({ IDENTITY: { "movePrecompileToAddress": RELOCATED } }),
    );
    let response = call_http(context, body).await;
    let output = frame_output(&response);

    assert!(
        output.contains("11223344"),
        "identity precompile did not run at the relocated address; frame output: {output}"
    );
}

/// And the other half of a move: the vacated address becomes an ordinary (empty)
/// account, so the traced call succeeds returning no data.
#[tokio::test]
async fn trace_call_treats_a_vacated_precompile_address_as_a_normal_account() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = trace_call(
        IDENTITY,
        json!({ IDENTITY: { "movePrecompileToAddress": RELOCATED } }),
    );
    let response = call_http(context, body).await;
    let output = frame_output(&response);

    assert!(
        !output.contains("11223344"),
        "vacated address still ran the identity precompile; frame output: {output}"
    );
}
