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

// -- ethSimulate-set-read-storage: deploy contract via code override, write+read storage
#[tokio::test]
async fn test_simulate_set_read_storage() {
    let ctx = test_context().await;
    // Contract with store(uint256) and retrieve() functions
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x608060405234801561001057600080fd5b50600436106100365760003560e01c80632e64cec11461003b5780636057361d14610059575b600080fd5b610043610075565b60405161005091906100d9565b60405180910390f35b610073600480360381019061006e919061009d565b61007e565b005b60008054905090565b8060008190555050565b60008135905061009781610103565b92915050565b6000602082840312156100b3576100b26100fe565b5b60006100c184828501610088565b91505092915050565b6100d3816100f4565b82525050565b60006020820190506100ee60008301846100ca565b92915050565b6000819050919050565b600080fd5b61010c816100f4565b811461011757600080fd5b5056fea2646970667358221220404e37f487a89a932dca5e77faaf6ca2de3b991f93d230604b1b8daaef64766264736f6c63430008070033"
                    }
                },
                "calls": [
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc200000000000000000000000000000000000000",
                        "input": "0x6057361d0000000000000000000000000000000000000000000000000000000000000005"
                    },
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc200000000000000000000000000000000000000",
                        "input": "0x2e64cec1"
                    }
                ]
            }]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let calls = result[0]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    // store(5) succeeds
    assert_eq!(calls[0]["status"], "0x1");
    // retrieve() returns 5
    assert_eq!(calls[1]["status"], "0x1");
    assert_eq!(
        calls[1]["returnData"],
        "0x0000000000000000000000000000000000000000000000000000000000000005"
    );
}

// -- ethSimulate-logs: contract emits a log
#[tokio::test]
async fn test_simulate_contract_logs() {
    let ctx = test_context().await;
    // Contract that emits LOG1 with topic 0xfff...fff
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff80600080a1600080f3"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc200000000000000000000000000000000000000",
                    "input": "0x6057361d0000000000000000000000000000000000000000000000000000000000000005"
                }]
            }]
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
        "0xc200000000000000000000000000000000000000"
    );
    assert_eq!(
        logs[0]["topics"][0],
        "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    );

    // logsBloom should NOT be all zeros (real contract log included)
    let bloom = result[0]["logsBloom"].as_str().unwrap();
    assert_ne!(bloom, format!("0x{}", "0".repeat(512)));
}

// -- ethSimulate-no-fields-call: call with empty object {}
#[tokio::test]
async fn test_simulate_no_fields_call() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "calls": [{}]
            }]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let calls = result[0]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    // A call with no fields should succeed (defaults applied)
    assert_eq!(calls[0]["status"], "0x1");
}

// -- ethSimulate-move-two-accounts-to-same-38023: duplicate movePrecompileToAddress target
#[tokio::test]
async fn test_simulate_move_precompile_duplicate_target() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0x0000000000000000000000000000000000000001": {
                        "movePrecompileToAddress": "0x0000000000000000000000000000000000123456"
                    },
                    "0x0000000000000000000000000000000000000002": {
                        "movePrecompileToAddress": "0x0000000000000000000000000000000000123456"
                    }
                }
            }]
        }, "latest"]"#,
    )
    .await;

    match result {
        Err(RpcErr::Simulate { code, .. }) => assert_eq!(code, -38023),
        other => panic!("expected Simulate error -38023, got: {other:?}"),
    }
}

// -- ethSimulate-simple-state-diff: state override with full `state` replacement across blocks
#[tokio::test]
async fn test_simulate_state_override_full_replacement() {
    let ctx = test_context().await;
    // Contract that reads slot 0 and slot 1
    // Block 1: write to both slots via calls
    // Block 2: override with `state` (wipes slot 1, sets slot 0 to new value)
    // Then read both slots — slot 0 should have override value, slot 1 should be 0
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [
                {
                    "stateOverrides": {
                        "0xc000000000000000000000000000000000000000": { "balance": "0x7d0" },
                        "0xc100000000000000000000000000000000000000": {
                            "code": "0x608060405234801561001057600080fd5b506004361061004c5760003560e01c80630ff4c916146100515780633033413b1461008157806344e12f871461009f5780637b8d56e3146100bd575b600080fd5b61006b600480360381019061006691906101f6565b6100d9565b6040516100789190610232565b60405180910390f35b61008961013f565b6040516100969190610232565b60405180910390f35b6100a7610145565b6040516100b49190610232565b60405180910390f35b6100d760048036038101906100d2919061024d565b61014b565b005b60006002821061011e576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610115906102ea565b60405180910390fd5b6000820361012c5760005490505b6001820361013a5760015490505b919050565b60015481565b60005481565b6002821061018e576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610185906102ea565b60405180910390fd5b600082036101a257806000819055506101b7565b600182036101b657806001819055506101b7565b5b5050565b600080fd5b6000819050919050565b6101d3816101c0565b81146101de57600080fd5b50565b6000813590506101f0816101ca565b92915050565b60006020828403121561020c5761020b6101bb565b5b600061021a848285016101e1565b91505092915050565b61022c816101c0565b82525050565b60006020820190506102476000830184610223565b92915050565b60008060408385031215610264576102636101bb565b5b6000610272858286016101e1565b9250506020610283858286016101e1565b9150509250929050565b600082825260208201905092915050565b7f746f6f2062696720736c6f740000000000000000000000000000000000000000600082015250565b60006102d4600c8361028d565b91506102df8261029e565b602082019050919050565b60006020820190508181036000830152610303816102c7565b905091905056fea2646970667358221220ceea194bb66b5b9f52c83e5bf5a1989255de8cb7157838eff98f970c3a04cb3064736f6c63430008120033"
                        }
                    },
                    "calls": [
                        {
                            "from": "0xc000000000000000000000000000000000000000",
                            "to": "0xc100000000000000000000000000000000000000",
                            "input": "0x7b8d56e300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001"
                        },
                        {
                            "from": "0xc000000000000000000000000000000000000000",
                            "to": "0xc100000000000000000000000000000000000000",
                            "input": "0x7b8d56e300000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002"
                        }
                    ]
                },
                {
                    "stateOverrides": {
                        "0xc100000000000000000000000000000000000000": {
                            "state": {
                                "0x0000000000000000000000000000000000000000000000000000000000000000": "0x1200000000000000000000000000000000000000000000000000000000000000"
                            }
                        }
                    },
                    "calls": [
                        {
                            "from": "0xc000000000000000000000000000000000000000",
                            "to": "0xc100000000000000000000000000000000000000",
                            "input": "0x0ff4c9160000000000000000000000000000000000000000000000000000000000000000"
                        },
                        {
                            "from": "0xc000000000000000000000000000000000000000",
                            "to": "0xc100000000000000000000000000000000000000",
                            "input": "0x0ff4c9160000000000000000000000000000000000000000000000000000000000000001"
                        }
                    ]
                }
            ]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let blocks = result.as_array().unwrap();
    assert_eq!(blocks.len(), 2);

    // Block 1: both writes succeed
    assert_eq!(blocks[0]["calls"][0]["status"], "0x1");
    assert_eq!(blocks[0]["calls"][1]["status"], "0x1");

    // Block 2: slot 0 was overridden to 0x12000...
    assert_eq!(blocks[1]["calls"][0]["status"], "0x1");
    assert_eq!(
        blocks[1]["calls"][0]["returnData"],
        "0x1200000000000000000000000000000000000000000000000000000000000000"
    );

    // Block 2: slot 1 was wiped by `state` override (full replacement) → returns 0
    assert_eq!(blocks[1]["calls"][1]["status"], "0x1");
    assert_eq!(
        blocks[1]["calls"][1]["returnData"],
        "0x0000000000000000000000000000000000000000000000000000000000000000"
    );
}

// -- ethSimulate-two-blocks-with-complete-eth-sends: multi-block with traceTransfers
#[tokio::test]
async fn test_simulate_two_blocks_trace_transfers() {
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
            ],
            "traceTransfers": true
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let blocks = result.as_array().unwrap();
    assert_eq!(blocks.len(), 2);

    // Block 1: transfer logs present
    let logs1 = blocks[0]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(logs1.len(), 1);
    assert_eq!(
        logs1[0]["address"],
        "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    );

    // Block 2: transfer logs present (c1 → c2)
    let logs2 = blocks[1]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(logs2.len(), 1);
    assert_eq!(
        logs2[0]["address"],
        "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    );

    // Second block's parentHash should match first block's hash
    assert_eq!(blocks[1]["parentHash"], blocks[0]["hash"]);
}

// -- ethSimulate-simple-send-from-contract: contract address as sender
// Per spec, sender-is-EOA check should be skipped in simulate mode.
#[tokio::test]
#[ignore = "bug: DefaultHook rejects contract sender in simulate mode"]
async fn test_simulate_send_from_contract() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {
                        "code": "0x00",
                        "balance": "0x3e8"
                    }
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
    // Spec: sending from contract should succeed in simulate (no EOA check)
    assert_eq!(calls[0]["status"], "0x1");
}

// -- ethSimulate-contract-calls-itself: contract calling itself
// Same root cause — sender has code from override, DefaultHook rejects.
#[tokio::test]
#[ignore = "bug: DefaultHook rejects contract sender in simulate mode"]
async fn test_simulate_contract_calls_itself() {
    let ctx = test_context().await;
    let result = simulate(
        &ctx,
        r#"[{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {
                        "code": "0x608060405234801561001057600080fd5b506000366060484641444543425a3a60014361002c919061009b565b406040516020016100469a99989796959493929190610138565b6040516020818303038152906040529050915050805190602001f35b6000819050919050565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052601160045260246000fd5b60006100a682610062565b91506100b183610062565b92508282039050818111156100c9576100c861006c565b5b92915050565b6100d881610062565b82525050565b600073ffffffffffffffffffffffffffffffffffffffff82169050919050565b6000610109826100de565b9050919050565b610119816100fe565b82525050565b6000819050919050565b6101328161011f565b82525050565b60006101408201905061014e600083018d6100cf565b61015b602083018c6100cf565b610168604083018b610110565b610175606083018a6100cf565b61018260808301896100cf565b61018f60a08301886100cf565b61019c60c08301876100cf565b6101a960e08301866100cf565b6101b76101008301856100cf565b6101c5610120830184610129565b9b9a505050505050505050505056fea26469706673582212205139ae3ba8d46d11c29815d001b725f9840c90e330884ed070958d5af4813d8764736f6c63430008120033"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc000000000000000000000000000000000000000"
                }]
            }]
        }, "latest"]"#,
    )
    .await
    .unwrap();

    let calls = result[0]["calls"].as_array().unwrap();
    assert_eq!(calls[0]["status"], "0x1");
    let return_data = calls[0]["returnData"].as_str().unwrap();
    assert!(return_data.len() > 2);
}
