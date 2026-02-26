//! Serialization round-trip tests for debugger types.

use bytes::Bytes;
use ethrex_common::{Address, U256};

use crate::types::{ReplayConfig, ReplayTrace, StepRecord};

#[test]
fn step_record_serializes() {
    let step = StepRecord {
        step_index: 0,
        pc: 10,
        opcode: 0x01,
        depth: 0,
        gas_remaining: 99994,
        stack_top: vec![U256::from(7), U256::from(3)],
        stack_depth: 2,
        memory_size: 0,
        code_address: Address::zero(),
    };
    let json = serde_json::to_value(&step).expect("StepRecord should serialize");
    assert_eq!(json["step_index"], 0);
    assert_eq!(json["pc"], 10);
    assert_eq!(json["opcode"], 1);
    assert_eq!(json["gas_remaining"], 99994);
    assert_eq!(json["stack_depth"], 2);
    assert_eq!(json["memory_size"], 0);
}

#[test]
fn replay_trace_serializes() {
    let trace = ReplayTrace {
        steps: vec![StepRecord {
            step_index: 0,
            pc: 0,
            opcode: 0x00,
            depth: 0,
            gas_remaining: 21000,
            stack_top: vec![],
            stack_depth: 0,
            memory_size: 0,
            code_address: Address::zero(),
        }],
        config: ReplayConfig::default(),
        gas_used: 21000,
        success: true,
        output: Bytes::new(),
    };
    let json = serde_json::to_value(&trace).expect("ReplayTrace should serialize");
    assert_eq!(json["gas_used"], 21000);
    assert_eq!(json["success"], true);
    assert!(json["steps"].is_array());
    assert_eq!(json["steps"].as_array().expect("steps array").len(), 1);
}

#[test]
fn replay_config_serializes() {
    let config = ReplayConfig::default();
    let json = serde_json::to_value(&config).expect("ReplayConfig should serialize");
    assert_eq!(json["stack_top_capture"], 8);
}

#[test]
fn step_record_fields() {
    let step = StepRecord {
        step_index: 42,
        pc: 100,
        opcode: 0x60,
        depth: 1,
        gas_remaining: 50000,
        stack_top: vec![U256::from(0xff)],
        stack_depth: 5,
        memory_size: 64,
        code_address: Address::from_low_u64_be(0x42),
    };
    let json = serde_json::to_string(&step).expect("should serialize");
    for field in [
        "step_index",
        "pc",
        "opcode",
        "depth",
        "gas_remaining",
        "stack_top",
        "stack_depth",
        "memory_size",
        "code_address",
    ] {
        assert!(json.contains(field), "missing field: {field}");
    }
}
