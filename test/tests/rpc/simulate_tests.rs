//! Integration tests for `eth_simulateV1`, ported from execution-apis test vectors.
//! State is set up entirely via `stateOverrides` so no historical chain dependency.

use ethrex_rpc::eth::simulate::SimulateV1Request;
use ethrex_rpc::rpc::{RpcApiContext, RpcHandler};
use ethrex_rpc::test_utils::{default_context_with_storage, setup_store};
use ethrex_rpc::utils::RpcErr;
use serde_json::Value;

/// Helper: create an RpcApiContext with in-memory storage and genesis state.
async fn test_context() -> RpcApiContext {
    let store = setup_store().await;
    default_context_with_storage(store).await
}

/// Helper: parse a JSON-RPC request and call the handler directly.
async fn simulate(context: &RpcApiContext, params_json: &str) -> Result<Value, RpcErr> {
    let params: Vec<Value> = serde_json::from_str(params_json).unwrap();
    let req = SimulateV1Request::parse(&Some(params))?;
    req.handle(context.clone()).await
}

/// Helper: parse a hex string like "0x5208" to u64.
fn parse_hex_u64(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

// -- ethSimulate-empty: simulate one block with no calls
#[tokio::test]
async fn test_simulate_empty() {
    let ctx = test_context().await;
    let result = simulate(&ctx, r#"[{"blockStateCalls":[{}]}, "latest"]"#)
        .await
        .unwrap();

    let blocks = result.as_array().unwrap();
    assert_eq!(blocks.len(), 1);

    let block = &blocks[0];
    assert_eq!(block["calls"].as_array().unwrap().len(), 0);
    assert_eq!(block["baseFeePerGas"], "0x0");
    assert_eq!(block["gasUsed"], "0x0");
    assert!(block["hash"].is_string());
    assert!(block["stateRoot"].is_string());
    assert_eq!(block["transactions"].as_array().unwrap().len(), 0);
}

// -- ethSimulate-simple: two ETH transfers with state override
#[tokio::test]
async fn test_simulate_simple_transfer() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": { "balance": "0x3e8" }
                },
                "calls": [
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "value": "0x3e8"
                    },
                    {
                        "from": "0xc100000000000000000000000000000000000000",
                        "to": "0xc200000000000000000000000000000000000000",
                        "value": "0x3e8"
                    }
                ]
            }]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let blocks = result.as_array().unwrap();
    assert_eq!(blocks.len(), 1);

    let calls = blocks[0]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["status"], "0x1");
    assert_eq!(calls[0]["gasUsed"], "0x5208"); // 21000
    assert_eq!(calls[1]["status"], "0x1");
    assert_eq!(calls[1]["gasUsed"], "0x5208");

    // Block should have 2 transactions
    assert_eq!(blocks[0]["transactions"].as_array().unwrap().len(), 2);
    assert_eq!(blocks[0]["gasUsed"], "0xa410"); // 42000
}

// -- ethSimulate-block-num-order-38020: block numbers must increase
#[tokio::test]
async fn test_simulate_block_number_order_error() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [
                { "blockOverrides": { "number": "0x91" } },
                { "blockOverrides": { "number": "0x87" } }
            ]
        }, "latest"]"#,
    )
    .await;

    match result {
        Err(RpcErr::Simulate { code, .. }) => assert_eq!(code, -38020),
        other => panic!("expected Simulate error -38020, got: {other:?}"),
    }
}

// -- ethSimulate-block-timestamp-order-38021: timestamps must not decrease
#[tokio::test]
async fn test_simulate_timestamp_order_error() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [
                { "blockOverrides": { "time": "0x9999999" } },
                { "blockOverrides": { "time": "0x1" } }
            ]
        }, "latest"]"#,
    )
    .await;

    match result {
        Err(RpcErr::Simulate { code, .. }) => assert_eq!(code, -38021),
        other => panic!("expected Simulate error -38021, got: {other:?}"),
    }
}

// -- ethSimulate-eth-send-should-produce-logs: traceTransfers
#[tokio::test]
async fn test_simulate_trace_transfers() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": { "balance": "0x7d0" }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x3e8"
                }]
            }],
            "traceTransfers": true
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let calls = result[0]["calls"].as_array().unwrap();
    assert_eq!(calls[0]["status"], "0x1");

    let logs = calls[0]["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        logs[0]["address"],
        "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    );
    assert_eq!(
        logs[0]["topics"][0],
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
    );

    // logsBloom should be all zeros (ETH transfer logs excluded)
    let bloom = result[0]["logsBloom"].as_str().unwrap();
    assert_eq!(bloom, format!("0x{}", "0".repeat(512)));
}

// -- ethSimulate-eth-send-should-not-produce-logs-by-default
#[tokio::test]
async fn test_simulate_no_trace_transfers_by_default() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": { "balance": "0x7d0" }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x3e8"
                }]
            }]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let calls = result[0]["calls"].as_array().unwrap();
    assert_eq!(calls[0]["status"], "0x1");
    assert_eq!(calls[0]["logs"].as_array().unwrap().len(), 0);
}

// -- ethSimulate-transfer-over-BlockStateCalls: state persists across blocks
#[tokio::test]
async fn test_simulate_cross_block_state() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [
                {
                    "stateOverrides": {
                        "0xc000000000000000000000000000000000000000": { "balance": "0x7d0" }
                    },
                    "calls": [{
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "value": "0x3e8"
                    }]
                },
                {
                    "calls": [{
                        "from": "0xc100000000000000000000000000000000000000",
                        "to": "0xc200000000000000000000000000000000000000",
                        "value": "0x3e8"
                    }]
                }
            ]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let blocks = result.as_array().unwrap();
    assert_eq!(blocks.len(), 2);

    // Block 1: transfer from c0 to c1 succeeds
    assert_eq!(blocks[0]["calls"][0]["status"], "0x1");

    // Block 2: transfer from c1 to c2 succeeds (c1 got funds from block 1)
    assert_eq!(blocks[1]["calls"][0]["status"], "0x1");
}

// -- ethSimulate-move-to-address-itself-reference-38022
#[tokio::test]
async fn test_simulate_move_precompile_self_reference() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0x0000000000000000000000000000000000000001": {
                        "movePrecompileToAddress": "0x0000000000000000000000000000000000000001"
                    }
                }
            }]
        }, "latest"]"#,
    )
    .await;

    match result {
        Err(RpcErr::Simulate { code, .. }) => assert_eq!(code, -38022),
        other => panic!("expected Simulate error -38022, got: {other:?}"),
    }
}

// -- ethSimulate-override-address-twice-in-separate-BlockStateCalls
#[tokio::test]
async fn test_simulate_override_across_blocks() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [
                {
                    "stateOverrides": {
                        "0xc000000000000000000000000000000000000000": { "balance": "0x7d0" }
                    },
                    "calls": [{
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "value": "0x3e8"
                    }]
                },
                {
                    "stateOverrides": {
                        "0xc000000000000000000000000000000000000000": { "balance": "0x7d0" }
                    },
                    "calls": [{
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "value": "0x3e8"
                    }]
                }
            ]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let blocks = result.as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    // Both blocks should succeed — balance re-overridden in block 2
    assert_eq!(blocks[0]["calls"][0]["status"], "0x1");
    assert_eq!(blocks[1]["calls"][0]["status"], "0x1");
}

// -- ethSimulate-blocknumber-increment: auto-increment
#[tokio::test]
async fn test_simulate_block_number_auto_increment() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{}, {}, {}]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let blocks = result.as_array().unwrap();
    assert_eq!(blocks.len(), 3);

    let n0 = parse_hex_u64(blocks[0]["number"].as_str().unwrap());
    let n1 = parse_hex_u64(blocks[1]["number"].as_str().unwrap());
    let n2 = parse_hex_u64(blocks[2]["number"].as_str().unwrap());

    assert_eq!(n1, n0 + 1);
    assert_eq!(n2, n1 + 1);
}

// -- ethSimulate-block-timestamp-auto-increment
#[tokio::test]
async fn test_simulate_timestamp_auto_increment() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{}, {}, {}]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let blocks = result.as_array().unwrap();
    let t0 = parse_hex_u64(blocks[0]["timestamp"].as_str().unwrap());
    let t1 = parse_hex_u64(blocks[1]["timestamp"].as_str().unwrap());
    let t2 = parse_hex_u64(blocks[2]["timestamp"].as_str().unwrap());

    // Default increment is 12s
    assert_eq!(t1, t0 + 12);
    assert_eq!(t2, t1 + 12);
}

// -- returnFullTransactions toggle
#[tokio::test]
async fn test_simulate_return_full_transactions() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": { "balance": "0x3e8" }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x1"
                }]
            }],
            "returnFullTransactions": true
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let txs = result[0]["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 1);
    // Full transaction objects have a "type" field
    assert!(txs[0]["type"].is_string());
    assert!(txs[0]["hash"].is_string());
}

// -- returnFullTransactions: false returns tx hashes (default)
#[tokio::test]
async fn test_simulate_return_tx_hashes() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": { "balance": "0x3e8" }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x1"
                }]
            }]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let txs = result[0]["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 1);
    // Default mode returns just tx hashes (strings)
    assert!(txs[0].is_string());
    let hash = txs[0].as_str().unwrap();
    assert!(hash.starts_with("0x"));
    assert_eq!(hash.len(), 66);
}
