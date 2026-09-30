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
/// Address the subcall tests install bytecode at.
const CALLEE: &str = "0x000000000000000000000000000000000000cafe";

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
    let response = call_http(&context, body).await;

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
    let response = call_http(&context, body).await;

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
    let response = call_http(&context, body).await;
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
    let response = call_http(&context, body).await;
    let output = frame_output(&response);

    assert!(
        !output.contains("11223344"),
        "vacated address still ran the identity precompile; frame output: {output}"
    );
}

// ---------------------------------------------------------------------------
// Dispatch from inside contract bytecode.
//
// Every test above calls the relocated address at the *top level*, which reaches only
// the dispatch in `vm.rs`. The CALL family dispatches separately, in
// `opcode_handlers/system.rs`, and the branch patched two expressions there: the
// `address_is_precompile` predicate and the `effective_precompile_address` resolution.
// Reaching a moved precompile through `STATICCALL` is the only way to exercise them.
// ---------------------------------------------------------------------------

/// PUSH4 0x11223344, MSTORE at 0, STATICCALL(gas, `target`, 0, 32, 32, 32), POP,
/// RETURN mem[32..64].
///
/// Calls `target` with one 32-byte word and returns whatever it echoed back, so the
/// identity precompile running there is visible as a word ending in `11223344` and its
/// absence as a word of zeros (an empty account's `STATICCALL` succeeds writing nothing,
/// and memory reads zero). `target` is a `PUSH2` literal, so it must fit in two bytes.
fn staticcall_and_return(target: u16) -> String {
    format!("0x6311223344600052602060206020600061{target:04x}5afa5060206020f3")
}

fn eth_call_no_data(to: &str, overrides: serde_json::Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_call",
        "params": [
            { "from": CALLER, "to": to, "data": "0x" },
            "latest",
            overrides
        ]
    })
    .to_string()
}

async fn call_result(overrides: serde_json::Value) -> String {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;
    let response = call_http(&context, eth_call_no_data(CALLEE, overrides)).await;
    response
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or_else(|| panic!("expected a string result, got: {response}"))
        .to_owned()
}

/// Control: with no relocation, `STATICCALL` to 0x04 reaches the identity precompile.
/// Without this the negative assertion in the vacated-address test below could pass
/// simply because the bytecode never worked.
#[tokio::test]
async fn subcall_reaches_the_identity_precompile_at_its_own_address() {
    let result = call_result(json!({
        CALLEE: { "code": staticcall_and_return(0x0004) }
    }))
    .await;

    assert!(
        result.contains("11223344"),
        "the STATICCALL harness itself is broken: {result}"
    );
}

/// A relocated precompile must dispatch when reached from contract bytecode, which goes
/// through `system.rs` rather than the top-level path. Exercises both patched
/// expressions: the destination has to *be* a precompile, and it has to resolve back to
/// the implementation at 0x04.
#[tokio::test]
async fn subcall_dispatches_a_moved_precompile_at_the_destination() {
    let result = call_result(json!({
        CALLEE: { "code": staticcall_and_return(0x0aaa) },
        IDENTITY: { "movePrecompileToAddress": RELOCATED }
    }))
    .await;

    assert!(
        result.contains("11223344"),
        "identity precompile did not run at the relocated address via STATICCALL: {result}"
    );
}

/// And the vacated address must stop being a precompile at this dispatch site too — the
/// `address_is_precompile` half on its own, since no resolution is involved.
#[tokio::test]
async fn subcall_treats_a_vacated_precompile_address_as_a_normal_account() {
    let result = call_result(json!({
        CALLEE: { "code": staticcall_and_return(0x0004) },
        IDENTITY: { "movePrecompileToAddress": RELOCATED }
    }))
    .await;

    assert!(
        !result.contains("11223344"),
        "vacated address still ran the identity precompile via STATICCALL: {result}"
    );
}

/// geth's `StateOverride.Apply` does `delete(precompiles, addr)` for **any** overridden
/// address, not just the source of a move. So overriding nothing but a precompile's
/// balance takes it out of the active precompile set and leaves an ordinary account.
///
/// This is the one semantic here that bites a request nobody wrote deliberately:
/// `{"0x04": {"balance": "0x1"}}` reads like a no-op but silences the precompile.
#[tokio::test]
async fn overriding_a_precompile_address_stops_it_dispatching() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = eth_call(IDENTITY, json!({ IDENTITY: { "balance": "0x1" } }));
    let response = call_http(&context, body).await;
    let result = response
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or_else(|| panic!("expected a string result, got: {response}"));

    assert!(
        !result.contains("11223344"),
        "an overridden precompile address must stop dispatching; got: {result}"
    );
}

/// Control: without the override the identity precompile still echoes, so the assertion
/// above tracks the override rather than a broken request.
#[tokio::test]
async fn an_unoverridden_precompile_still_dispatches() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = eth_call(IDENTITY, json!({}));
    let response = call_http(&context, body).await;
    let result = response
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or_else(|| panic!("expected a string result, got: {response}"));

    assert!(
        result.contains("11223344"),
        "the identity precompile should echo when nothing overrides it; got: {result}"
    );
}
