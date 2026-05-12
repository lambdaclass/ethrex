//! Wire-format tests for the EIP-3155 streaming serializer in
//! `ethrex_common::tracing` — pins each per-step / summary / state-root field
//! against a captured `evm v1.17.3 run --json` reference.

use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::tracing::{
    MemoryChunk, OpcodeStep, StreamingOpts, write_streaming_state_root, write_streaming_step,
    write_streaming_summary,
};

// Mirrors the third step of `evm v1.17.3 run --json 0x6001600101`, used to
// anchor byte-exact format parity with the geth reference output.
fn add_step() -> OpcodeStep {
    OpcodeStep {
        pc: 4,
        op: 0x01,           // ADD
        gas: 9_999_999_994, // 0x2540be3fa
        gas_cost: 3,
        mem_size: 0,
        depth: 1,
        return_data: Bytes::new(),
        refund: 0,
        stack: Some(vec![U256::one(), U256::one()]),
        memory: None,
        storage: None,
        error: None,
    }
}

// 1.4a — ADD step byte-exact match against the streaming format
#[test]
fn test_1_4a_streaming_add_step() {
    let step = add_step();
    let opts = StreamingOpts::default();
    let mut buf = Vec::new();
    write_streaming_step(&mut buf, &step, &opts).unwrap();
    let output = String::from_utf8(buf).unwrap();
    let expected = "{\"pc\":4,\"op\":1,\"gas\":\"0x2540be3fa\",\"gasCost\":\"0x3\",\"memSize\":0,\"stack\":[\"0x1\",\"0x1\"],\"depth\":1,\"refund\":0,\"opName\":\"ADD\"}\n";
    assert_eq!(output, expected, "streaming ADD step mismatch");
}

// 1.4b — MSTORE step with memory enabled; memory reassembled as single hex blob
#[test]
fn test_1_4b_streaming_memory() {
    let step = OpcodeStep {
        pc: 0,
        op: 0x52, // MSTORE
        gas: 100,
        gas_cost: 3,
        mem_size: 64,
        depth: 1,
        return_data: Bytes::new(),
        refund: 0,
        stack: None,
        memory: Some(vec![
            MemoryChunk([0u8; 32]),
            MemoryChunk({
                let mut b = [0u8; 32];
                b[31] = 0x01;
                b
            }),
        ]),
        storage: None,
        error: None,
    };
    let opts = StreamingOpts {
        disable_stack: true,
        disable_memory: false,
        ..StreamingOpts::default()
    };
    let mut buf = Vec::new();
    write_streaming_step(&mut buf, &step, &opts).unwrap();
    let output = String::from_utf8(buf).unwrap();
    // 32 zero bytes + 31 zero bytes + 0x01
    let expected_mem = format!("0x{}{}01", "00".repeat(32), "00".repeat(31));
    assert!(
        output.contains(&format!("\"memory\":\"{}\"", expected_mem)),
        "memory field mismatch, got: {output}"
    );
    // confirm it is a single string, not an array
    assert!(
        !output.contains("\"memory\":["),
        "memory must not be an array"
    );
}

// 1.4c — REVERT step with error field
#[test]
fn test_1_4c_streaming_error() {
    let step = OpcodeStep {
        pc: 10,
        op: 0xfd, // REVERT
        gas: 500,
        gas_cost: 0,
        mem_size: 0,
        depth: 1,
        return_data: Bytes::new(),
        refund: 0,
        stack: Some(vec![]),
        memory: None,
        storage: None,
        error: Some("execution reverted".to_string()),
    };
    let opts = StreamingOpts::default();
    let mut buf = Vec::new();
    write_streaming_step(&mut buf, &step, &opts).unwrap();
    let output = String::from_utf8(buf).unwrap();
    assert!(
        output.contains("\"error\":\"execution reverted\""),
        "error field missing or wrong, got: {output}"
    );
}

// 1.4d — summary line, no error
#[test]
fn test_1_4d_streaming_summary_no_error() {
    let mut buf = Vec::new();
    write_streaming_summary(&mut buf, &[0xde, 0xad], 42, None).unwrap();
    let output = String::from_utf8(buf).unwrap();
    let expected = "{\"output\":\"dead\",\"gasUsed\":\"0x2a\"}\n";
    assert_eq!(output, expected, "summary no-error mismatch");
}

// 1.4e — summary line with error
#[test]
fn test_1_4e_streaming_summary_with_error() {
    let mut buf = Vec::new();
    write_streaming_summary(&mut buf, &[], 0, Some("out of gas")).unwrap();
    let output = String::from_utf8(buf).unwrap();
    let expected = "{\"output\":\"\",\"gasUsed\":\"0x0\",\"error\":\"out of gas\"}\n";
    assert_eq!(output, expected, "summary with-error mismatch");
}

// 1.4f — disable_stack omits stack field
#[test]
fn test_1_4f_disable_stack() {
    let step = add_step();
    let opts = StreamingOpts {
        disable_stack: true,
        ..StreamingOpts::default()
    };
    let mut buf = Vec::new();
    write_streaming_step(&mut buf, &step, &opts).unwrap();
    let output = String::from_utf8(buf).unwrap();
    assert!(
        !output.contains("\"stack\""),
        "stack should be absent when disable_stack=true, got: {output}"
    );
}

// 1.4g — unknown opcode 0xee
#[test]
fn test_1_4g_unknown_opcode() {
    let step = OpcodeStep {
        pc: 0,
        op: 0xee,
        gas: 100,
        gas_cost: 0,
        mem_size: 0,
        depth: 1,
        return_data: Bytes::new(),
        refund: 0,
        stack: Some(vec![]),
        memory: None,
        storage: None,
        error: None,
    };
    let opts = StreamingOpts::default();
    let mut buf = Vec::new();
    write_streaming_step(&mut buf, &step, &opts).unwrap();
    let output = String::from_utf8(buf).unwrap();
    assert!(
        output.contains("\"op\":238"),
        "op should be decimal 238 for 0xee, got: {output}"
    );
    assert!(
        output.contains("\"opName\":\"opcode 0xee not defined\""),
        "opName mismatch for unknown opcode, got: {output}"
    );
}

// 1.4h — write_streaming_state_root with H256::zero(); the colon-space is
// required because goevmlab does a literal byte search for `"stateRoot": "`.
#[test]
fn test_1_4h_state_root() {
    let mut buf = Vec::new();
    write_streaming_state_root(&mut buf, H256::zero()).unwrap();
    let output = String::from_utf8(buf).unwrap();
    let expected =
        "{\"stateRoot\": \"0x0000000000000000000000000000000000000000000000000000000000000000\"}\n";
    assert_eq!(output, expected, "state root line mismatch");
    assert!(
        output.contains("\"stateRoot\": \""),
        "must contain colon-space after stateRoot key"
    );
}

// 1.5 — snapshot test: existing Serialize for OpcodeStep (legacy RPC shape).
// Pinning this guards against accidental drift in the RPC `debug_traceTransaction`
// wire format while we evolve the streaming serializer alongside it.
#[test]
fn test_1_5_legacy_rpc_serialize_snapshot() {
    let step = add_step();
    let json = serde_json::to_string(&step).unwrap();
    // Legacy shape: op is mnemonic string, gas/gasCost/refund are numeric,
    // memSize is numeric, returnData is "0x", stack is array of hex strings.
    let expected = r#"{"pc":4,"op":"ADD","gas":9999999994,"gasCost":3,"depth":1,"stack":["0x1","0x1"],"memSize":0,"returnData":"0x","refund":0}"#;
    assert_eq!(
        json, expected,
        "legacy RPC OpcodeStep serialization changed"
    );
}
