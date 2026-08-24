//! Parse-layer tests for `debug_traceCall`'s parameter handling
//! ([`TraceCallRequest::parse`]): optional `null` arguments and the explicit
//! rejection of the unsupported `stateOverrides` / `blockOverrides` fields.

use ethrex_rpc::rpc::RpcHandler;
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

/// `stateOverrides` is unsupported and must be rejected explicitly rather than
/// silently ignored.
#[test]
fn trace_call_rejects_state_overrides() {
    let result = parse(vec![
        call_object(),
        json!("latest"),
        json!({ "stateOverrides": { "0x1000000000000000000000000000000000000000": { "balance": "0x1" } } }),
    ]);
    assert!(
        matches!(result, Err(RpcErr::BadParams(_))),
        "stateOverrides must be rejected with BadParams"
    );
}

/// `blockOverrides` is unsupported and must be rejected explicitly.
#[test]
fn trace_call_rejects_block_overrides() {
    let result = parse(vec![
        call_object(),
        json!("latest"),
        json!({ "blockOverrides": { "number": "0x1" } }),
    ]);
    assert!(
        matches!(result, Err(RpcErr::BadParams(_))),
        "blockOverrides must be rejected with BadParams"
    );
}
