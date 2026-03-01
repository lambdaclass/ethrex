// Tests for the eth_simulateV1 RPC endpoint.
//
// These tests are based on the ethereum/execution-apis rpc-compat test suite
// (https://github.com/ethereum/execution-apis/tree/main/tests/eth_simulateV1)
// but adapted for ethrex's post-merge-only chain (all forks active from block 0,
// terminalTotalDifficultyPassed=true), since the upstream tests rely on a
// pre-merge chain that ethrex does not support.
//
// Genesis used: fixtures/genesis/l1.json
//   chainId: 9, all forks at block 0, Prague active
//   baseFeePerGas: 1 Gwei (0x3b9aca00, EIP-1559 default)
//   genesis timestamp: 1718040081
//
// Funded EOAs from genesis alloc:
//   SENDER_A = 0x00000a8d3f37af8def18832962ee008d8dca4f7b
//   SENDER_B = 0x00002132ce94eefb06eb15898c1aabd94feb0ac2
//
// Clean addresses (no pre-existing state, used as call targets / contract slots):
//   0xc000000000000000000000000000000000000000
//   0xc100000000000000000000000000000000000000

use ethrex_rpc::{
    RpcErr, RpcHandler, SimulateV1Request,
    test_utils::{default_context_with_storage, setup_store},
};
use serde_json::{Value, json};

const SENDER_A: &str = "0x00000a8d3f37af8def18832962ee008d8dca4f7b";
const SENDER_B: &str = "0x00002132ce94eefb06eb15898c1aabd94feb0ac2";

async fn run_simulate(payload: Value) -> Result<Value, RpcErr> {
    let store = setup_store().await;
    let ctx = default_context_with_storage(store).await;
    let params = Some(vec![payload, json!("latest")]);
    SimulateV1Request::parse(&params)?.handle(ctx).await
}

fn hex_u64(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

// ── Parse-level tests (synchronous) ────────────────────────────────

#[test]
fn test_parse_no_params() {
    let result = SimulateV1Request::parse(&None);
    assert!(result.is_err());
}

#[test]
fn test_parse_empty_params() {
    let result = SimulateV1Request::parse(&Some(vec![]));
    assert!(result.is_err());
}

#[test]
fn test_parse_too_many_params() {
    let result = SimulateV1Request::parse(&Some(vec![
        json!({"blockStateCalls": []}),
        json!("latest"),
        json!("extra"),
    ]));
    assert!(result.is_err());
}

#[test]
fn test_parse_too_many_block_state_calls() {
    // 257 empty block state calls (max is 256)
    let calls: Vec<Value> = (0..257).map(|_| json!({})).collect();
    let params = Some(vec![json!({"blockStateCalls": calls}), json!("latest")]);
    let result = SimulateV1Request::parse(&params);
    assert!(result.is_err());
    let err_msg = result.err().unwrap().to_string();
    assert!(
        err_msg.contains("Too many") || err_msg.contains("too many"),
        "expected 'Too many' in: {err_msg}"
    );
}

#[test]
fn test_parse_state_and_state_diff_mutually_exclusive() {
    // state and stateDiff are mutually exclusive per the spec
    let params = Some(vec![
        json!({
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {
                        "state": {
                            "0x0000000000000000000000000000000000000000000000000000000000000000": "0x0000000000000000000000000000000000000000000000000000000000000001"
                        },
                        "stateDiff": {
                            "0x0000000000000000000000000000000000000000000000000000000000000000": "0x0000000000000000000000000000000000000000000000000000000000000002"
                        }
                    }
                }
            }]
        }),
        json!("latest"),
    ]);
    let result = SimulateV1Request::parse(&params);
    assert!(result.is_err());
}

// ── Async functional tests ──────────────────────────────────────────

// Ref: ethSimulate-empty.io
#[tokio::test]
async fn test_empty_block_state_calls() {
    let result = run_simulate(json!({"blockStateCalls": [{}]})).await;
    let blocks = result.unwrap();
    let arr = blocks.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["calls"].as_array().unwrap().len(), 0);
    assert_eq!(hex_u64(&arr[0]["number"]), 1);
}

// Ref: ethSimulate-simple.io
#[tokio::test]
async fn test_simple_eth_transfer() {
    let contract = "0xc000000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "stateOverrides": {
                SENDER_A: {"balance": "0xde0b6b3a7640000"}
            },
            "calls": [{
                "from": SENDER_A,
                "to": contract,
                "value": "0x1"
            }]
        }]
    }))
    .await;
    let blocks = result.unwrap();
    let arr = blocks.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let calls = arr[0]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(hex_u64(&calls[0]["status"]), 1);
    assert_eq!(calls[0]["logs"].as_array().unwrap().len(), 0);
    assert_eq!(calls[0]["returnData"].as_str().unwrap(), "0x");
    assert!(hex_u64(&calls[0]["gasUsed"]) > 0);
    assert_eq!(hex_u64(&arr[0]["number"]), 1);
}

// Ref: ethSimulate-two-blocks-with-complete-eth-sends.io
#[tokio::test]
async fn test_two_transfers_in_one_block() {
    let addr_a = "0xc000000000000000000000000000000000000000";
    let addr_b = "0xc100000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "stateOverrides": {
                SENDER_A: {"balance": "0xde0b6b3a7640000"}
            },
            "calls": [
                {"from": SENDER_A, "to": addr_a, "value": "0x1"},
                {"from": SENDER_A, "to": addr_b, "value": "0x1"}
            ]
        }]
    }))
    .await;
    let blocks = result.unwrap();
    let calls = blocks[0]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(hex_u64(&calls[0]["status"]), 1);
    assert_eq!(hex_u64(&calls[1]["status"]), 1);
}

// Ref: ethSimulate-simple-validation-fulltx.io
#[tokio::test]
async fn test_return_full_transactions_true() {
    let contract = "0xc000000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "calls": [{"from": SENDER_A, "to": contract}]
        }],
        "returnFullTransactions": true
    }))
    .await;
    let blocks = result.unwrap();
    let txs = blocks[0]["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 1);
    let tx = &txs[0];
    assert!(tx.is_object(), "expected transaction to be an object");
    assert!(tx.get("from").is_some(), "missing 'from' field");
    assert!(tx.get("to").is_some(), "missing 'to' field");
    assert!(tx.get("nonce").is_some(), "missing 'nonce' field");
    assert!(tx.get("type").is_some(), "missing 'type' field");
    // Simulated transactions are unsigned: v/r/s are zero
    assert_eq!(tx["v"].as_str().unwrap(), "0x0");
}

// Ref: ethSimulate-simple.io (default returnFullTransactions=false)
#[tokio::test]
async fn test_return_full_transactions_false() {
    let contract = "0xc000000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "calls": [{"from": SENDER_A, "to": contract}]
        }]
    }))
    .await;
    let blocks = result.unwrap();
    let txs = blocks[0]["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 1);
    let tx_hash = txs[0].as_str().unwrap();
    assert!(tx_hash.starts_with("0x"), "expected hash to start with 0x");
    assert_eq!(
        tx_hash.len(),
        66,
        "expected 32-byte hex hash (66 chars with 0x)"
    );
}

// Ref: ethSimulate-logs.io
// Bytecode: PUSH1 32 (size), PUSH1 0 (offset), LOG0 (no topics), STOP → 0x60206000a000
#[tokio::test]
async fn test_contract_code_override_emits_log() {
    let contract = "0xc000000000000000000000000000000000000000";
    let log0_code = "0x60206000a000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "stateOverrides": {
                contract: {"code": log0_code}
            },
            "calls": [{"from": SENDER_A, "to": contract}]
        }]
    }))
    .await;
    let blocks = result.unwrap();
    let calls = blocks[0]["calls"].as_array().unwrap();
    assert_eq!(hex_u64(&calls[0]["status"]), 1);
    let logs = calls[0]["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        logs[0]["address"].as_str().unwrap().to_lowercase(),
        contract.to_lowercase()
    );
    assert_eq!(logs[0]["logIndex"].as_str().unwrap(), "0x0");
    assert_eq!(logs[0]["removed"].as_bool().unwrap(), false);
}

// Ref: ethSimulate-set-read-storage.io
// Bytecode: PUSH1 0 SLOAD PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN → 0x60005460005260206000f3
#[tokio::test]
async fn test_storage_state_override() {
    let contract = "0xc000000000000000000000000000000000000000";
    let sload_code = "0x60005460005260206000f3";
    let storage_val = "0x0000000000000000000000000000000000000000000000000000000000000042";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "stateOverrides": {
                contract: {
                    "code": sload_code,
                    "state": {
                        "0x0000000000000000000000000000000000000000000000000000000000000000": storage_val
                    }
                }
            },
            "calls": [{"from": SENDER_A, "to": contract}]
        }]
    }))
    .await;
    let blocks = result.unwrap();
    let calls = blocks[0]["calls"].as_array().unwrap();
    assert_eq!(hex_u64(&calls[0]["status"]), 1);
    let return_data = calls[0]["returnData"].as_str().unwrap();
    assert!(
        return_data.ends_with("0000000000000000000000000000000000000000000000000000000000000042"),
        "expected returnData to end with 0x42 word, got: {return_data}"
    );
}

// Ref: ethSimulate-simple-state-diff.io
#[tokio::test]
async fn test_storage_state_diff_override() {
    let contract = "0xc000000000000000000000000000000000000000";
    let sload_code = "0x60005460005260206000f3";
    let storage_val = "0x0000000000000000000000000000000000000000000000000000000000000042";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "stateOverrides": {
                contract: {
                    "code": sload_code,
                    "stateDiff": {
                        "0x0000000000000000000000000000000000000000000000000000000000000000": storage_val
                    }
                }
            },
            "calls": [{"from": SENDER_A, "to": contract}]
        }]
    }))
    .await;
    let blocks = result.unwrap();
    let calls = blocks[0]["calls"].as_array().unwrap();
    assert_eq!(hex_u64(&calls[0]["status"]), 1);
    let return_data = calls[0]["returnData"].as_str().unwrap();
    assert!(
        return_data.ends_with("0000000000000000000000000000000000000000000000000000000000000042"),
        "expected returnData to end with 0x42 word, got: {return_data}"
    );
}

// Ref: ethSimulate-override-block-num.io
#[tokio::test]
async fn test_block_number_override() {
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "blockOverrides": {"number": "0x64"}
        }]
    }))
    .await;
    let blocks = result.unwrap();
    let arr = blocks.as_array().unwrap();
    // Gap filling produces blocks 1..100; the last is the overridden one
    let last = arr.last().unwrap();
    assert_eq!(hex_u64(&last["number"]), 0x64);
}

// Ref: ethSimulate-block-timestamps-incrementing.io
#[tokio::test]
async fn test_timestamp_override() {
    // Genesis timestamp = 1718040081 = 0x664A0951; use a value well above it
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "blockOverrides": {"time": "0x70000000"}
        }]
    }))
    .await;
    let blocks = result.unwrap();
    let arr = blocks.as_array().unwrap();
    let last = arr.last().unwrap();
    assert_eq!(hex_u64(&last["timestamp"]), 0x70000000_u64);
}

// Ref: ethSimulate-blocknumber-increment.io
#[tokio::test]
async fn test_multiple_blocks() {
    let result = run_simulate(json!({
        "blockStateCalls": [{}, {}, {}]
    }))
    .await;
    let blocks = result.unwrap();
    let arr = blocks.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(hex_u64(&arr[0]["number"]), 1);
    assert_eq!(hex_u64(&arr[1]["number"]), 2);
    assert_eq!(hex_u64(&arr[2]["number"]), 3);
    let t0 = hex_u64(&arr[0]["timestamp"]);
    let t1 = hex_u64(&arr[1]["timestamp"]);
    let t2 = hex_u64(&arr[2]["timestamp"]);
    assert!(t1 > t0, "timestamps must be increasing: {t1} <= {t0}");
    assert!(t2 > t1, "timestamps must be increasing: {t2} <= {t1}");
}

// Ref: ethSimulate-add-more-non-defined-BlockStateCalls-than-fit.io
#[tokio::test]
async fn test_block_number_gap_filling() {
    // First block defaults to number=1, second is overridden to 5
    // → gap-fills blocks 2, 3, 4 → 5 total blocks in the response
    let result = run_simulate(json!({
        "blockStateCalls": [
            {},
            {"blockOverrides": {"number": "0x5"}}
        ]
    }))
    .await;
    let blocks = result.unwrap();
    let arr = blocks.as_array().unwrap();
    assert!(arr.len() >= 2, "expected at least 2 blocks");
    assert_eq!(
        hex_u64(&arr[0]["number"]),
        1,
        "first block should be number 1"
    );
    let last = arr.last().unwrap();
    assert_eq!(hex_u64(&last["number"]), 5, "last block should be number 5");
}

// Ref: ethSimulate-eth-send-should-produce-logs.io
#[tokio::test]
async fn test_trace_transfers_produces_log() {
    let recipient = "0xc000000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "stateOverrides": {
                SENDER_A: {"balance": "0xde0b6b3a7640000"}
            },
            "calls": [{
                "from": SENDER_A,
                "to": recipient,
                "value": "0x1"
            }]
        }],
        "traceTransfers": true
    }))
    .await;
    let blocks = result.unwrap();
    let calls = blocks[0]["calls"].as_array().unwrap();
    let logs = calls[0]["logs"].as_array().unwrap();
    assert!(!logs.is_empty(), "expected at least 1 trace transfer log");
    // Synthetic transfer logs are emitted from 0xeeee...eeee
    assert_eq!(
        logs[0]["address"].as_str().unwrap().to_lowercase(),
        "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    );
    // Transfer(address,address,uint256) has 3 topics
    assert_eq!(logs[0]["topics"].as_array().unwrap().len(), 3);
}

// Ref: ethSimulate-eth-send-should-not-produce-logs-by-default.io
#[tokio::test]
async fn test_trace_transfers_zero_value_no_log() {
    let recipient = "0xc000000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "calls": [{
                "from": SENDER_A,
                "to": recipient,
                "value": "0x0"
            }]
        }],
        "traceTransfers": true
    }))
    .await;
    let blocks = result.unwrap();
    let calls = blocks[0]["calls"].as_array().unwrap();
    // Zero-value transfers must not produce a synthetic log
    assert!(
        calls[0]["logs"].as_array().unwrap().is_empty(),
        "expected no trace transfer log for zero-value call"
    );
}

// Ref: ethSimulate-check-that-nonce-increases.io
#[tokio::test]
async fn test_nonce_auto_increments() {
    let recipient = "0xc000000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "stateOverrides": {
                SENDER_B: {"balance": "0xde0b6b3a7640000"}
            },
            "calls": [
                {"from": SENDER_B, "to": recipient, "value": "0x1"},
                {"from": SENDER_B, "to": recipient, "value": "0x1"}
            ]
        }],
        "returnFullTransactions": true
    }))
    .await;
    let blocks = result.unwrap();
    let txs = blocks[0]["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 2);
    assert_eq!(hex_u64(&txs[0]["nonce"]), 0, "first tx nonce should be 0");
    assert_eq!(hex_u64(&txs[1]["nonce"]), 1, "second tx nonce should be 1");
}

// Ref: ethSimulate-block-num-order-38020.io
#[tokio::test]
async fn test_block_number_order_error() {
    let result = run_simulate(json!({
        "blockStateCalls": [
            {"blockOverrides": {"number": "0x10"}},
            {"blockOverrides": {"number": "0x05"}}
        ]
    }))
    .await;
    assert!(result.is_err());
    match result.err().unwrap() {
        RpcErr::SimulateError { code, .. } => {
            assert_eq!(code, -38020);
        }
        other => panic!("expected SimulateError, got: {other:?}"),
    }
}

// Ref: ethSimulate-block-timestamp-order-38021.io
#[tokio::test]
async fn test_timestamp_order_error() {
    // Both timestamps must be above genesis (0x664A0951); second must be earlier than first
    let result = run_simulate(json!({
        "blockStateCalls": [
            {"blockOverrides": {"time": "0x70000000"}},
            {"blockOverrides": {"time": "0x6fffffff"}}
        ]
    }))
    .await;
    assert!(result.is_err());
    match result.err().unwrap() {
        RpcErr::SimulateError { code, .. } => {
            assert_eq!(code, -38021);
        }
        other => panic!("expected SimulateError, got: {other:?}"),
    }
}

// Ref: ethSimulate-gas-fees-and-value-error-38014-with-validation.io
#[tokio::test]
async fn test_validation_insufficient_funds_error() {
    // Address with no genesis balance + no balance override → insufficient funds
    let zero_balance_addr = "0xc000000000000000000000000000000000000000";
    let recipient = "0xc100000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "calls": [{
                "from": zero_balance_addr,
                "to": recipient,
                "value": "0x1",
                "maxFeePerGas": "0x3b9aca00",
                "maxPriorityFeePerGas": "0x0"
            }]
        }],
        "validation": true
    }))
    .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        RpcErr::SimulateError { code, .. } => {
            assert_eq!(code, -38014);
        }
        other => panic!("expected SimulateError, got: {other:?}"),
    }
}

// Ref: ethSimulate-simple-no-funds-with-validation.io
#[tokio::test]
async fn test_validation_success() {
    // SENDER_A has genesis balance → validation passes
    let recipient = "0xc000000000000000000000000000000000000000";
    let result = run_simulate(json!({
        "blockStateCalls": [{
            "calls": [{
                "from": SENDER_A,
                "to": recipient,
                "value": "0x1",
                "maxFeePerGas": "0x3b9aca00",
                "maxPriorityFeePerGas": "0x0"
            }]
        }],
        "validation": true
    }))
    .await;
    let blocks = result.unwrap();
    let calls = blocks[0]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(hex_u64(&calls[0]["status"]), 1);
}
