use serde_json::{Value, json};

use super::helpers::{rpc_call, rpc_call_expect_err, setup_single_transfer_block};

async fn trace_with(
    store: &ethrex_storage::Store,
    tx: ethrex_common::H256,
    config: Value,
) -> Value {
    rpc_call(
        store,
        "debug_traceTransaction",
        vec![json!(format!("{tx:#x}")), config],
    )
    .await
}

#[tokio::test]
async fn mux_tracer_call_subtracer_matches_standalone() {
    // The mux output's `callTracer` field must match what running
    // `callTracer` alone produces. This is the load-bearing correctness
    // invariant — anything subtler than equality means the multiplexer is
    // mutating sub-tracer behaviour.
    let env = setup_single_transfer_block().await;

    let mux = trace_with(
        &env.store,
        env.tx_hash,
        json!({
            "tracer": "muxTracer",
            "tracerConfig": { "callTracer": {"onlyTopCall": true} },
        }),
    )
    .await;
    let standalone = trace_with(
        &env.store,
        env.tx_hash,
        json!({
            "tracer": "callTracer",
            "tracerConfig": {"onlyTopCall": true},
        }),
    )
    .await;
    assert_eq!(
        mux["callTracer"], standalone,
        "muxTracer's callTracer slot must equal the standalone callTracer output"
    );
}

#[tokio::test]
async fn mux_tracer_prestate_subtracer_matches_standalone() {
    let env = setup_single_transfer_block().await;
    let mux = trace_with(
        &env.store,
        env.tx_hash,
        json!({
            "tracer": "muxTracer",
            "tracerConfig": { "prestateTracer": {} },
        }),
    )
    .await;
    let standalone = trace_with(&env.store, env.tx_hash, json!({"tracer": "prestateTracer"})).await;
    assert_eq!(mux["prestateTracer"], standalone);
}

#[tokio::test]
async fn mux_tracer_multiple_subtracers_each_match_standalone() {
    let env = setup_single_transfer_block().await;
    let mux = trace_with(
        &env.store,
        env.tx_hash,
        json!({
            "tracer": "muxTracer",
            "tracerConfig": {
                "callTracer": {},
                "prestateTracer": {"diffMode": true},
            },
        }),
    )
    .await;
    let call_standalone =
        trace_with(&env.store, env.tx_hash, json!({"tracer": "callTracer"})).await;
    let prestate_standalone = trace_with(
        &env.store,
        env.tx_hash,
        json!({
            "tracer": "prestateTracer",
            "tracerConfig": {"diffMode": true},
        }),
    )
    .await;
    assert_eq!(mux["callTracer"], call_standalone);
    assert_eq!(mux["prestateTracer"], prestate_standalone);
    assert_eq!(
        mux.as_object().unwrap().len(),
        2,
        "no spurious sub-tracer slots"
    );
}

#[tokio::test]
async fn mux_tracer_noop_returns_empty_object() {
    let env = setup_single_transfer_block().await;
    let mux = trace_with(
        &env.store,
        env.tx_hash,
        json!({
            "tracer": "muxTracer",
            "tracerConfig": { "noopTracer": {} },
        }),
    )
    .await;
    assert_eq!(
        mux["noopTracer"],
        json!({}),
        "noopTracer slot must be an empty object"
    );
}

#[tokio::test]
async fn mux_tracer_unknown_subtracer_errors() {
    let env = setup_single_transfer_block().await;
    let err = rpc_call_expect_err(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", env.tx_hash)),
            json!({
                "tracer": "muxTracer",
                "tracerConfig": { "bogusTracer": {} },
            }),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("unknown sub-tracer"),
        "expected unknown-sub-tracer error, got: {msg}"
    );
}

#[tokio::test]
async fn mux_tracer_missing_tracer_config_errors() {
    let env = setup_single_transfer_block().await;
    let err = rpc_call_expect_err(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", env.tx_hash)),
            json!({"tracer": "muxTracer"}),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("tracerConfig"),
        "expected missing-config error, got: {msg}"
    );
}

#[tokio::test]
async fn mux_tracer_block_level_returns_bad_params() {
    // debug_traceBlockByNumber with muxTracer is not supported. It must
    // surface as `BadParams` (user input) not `Internal` (server fault).
    let env = setup_single_transfer_block().await;
    let err = rpc_call_expect_err(
        &env.store,
        "debug_traceBlockByNumber",
        vec![
            json!(format!("{:#x}", env.block.header.number)),
            json!({
                "tracer": "muxTracer",
                "tracerConfig": { "callTracer": {} },
            }),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BadParams"),
        "block-level muxTracer must surface as BadParams, got: {msg}"
    );
    assert!(
        msg.contains("muxTracer"),
        "error message should mention muxTracer, got: {msg}"
    );
}
