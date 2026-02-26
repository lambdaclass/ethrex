//! Edge case tests for DebugRecorder's stack capture behavior.
//!
//! Tests:
//! - capture_stack_top with empty stack (0 items)
//! - capture_stack_top with fewer items than requested
//! - capture_stack_top with exact match
//! - capture_stack_top with custom config (stack_top_capture = 0)

use ethrex_common::{Address, U256};
use ethrex_levm::call_frame::Stack;
use ethrex_levm::debugger_hook::OpcodeRecorder;

use crate::recorder::DebugRecorder;
use crate::types::ReplayConfig;

#[test]
fn test_capture_empty_stack() {
    let config = ReplayConfig {
        stack_top_capture: 8,
    };
    let mut recorder = DebugRecorder::new(config);
    let stack = Stack::default(); // empty stack, len() == 0

    recorder.record_step(
        0x00, // STOP
        0,    // pc
        1_000_000,
        0,               // depth
        &stack,          // empty stack
        0,               // memory_size
        Address::zero(), // code_address
    );

    assert_eq!(recorder.steps.len(), 1);
    let step = &recorder.steps[0];
    assert!(
        step.stack_top.is_empty(),
        "empty stack should produce empty stack_top, got {:?}",
        step.stack_top
    );
    assert_eq!(step.stack_depth, 0);
}

#[test]
fn test_capture_stack_fewer_than_requested() {
    let config = ReplayConfig {
        stack_top_capture: 8, // request 8 items
    };
    let mut recorder = DebugRecorder::new(config);
    let mut stack = Stack::default();

    // Push only 3 items (fewer than the 8 requested)
    stack.push(U256::from(10u64)).expect("push");
    stack.push(U256::from(20u64)).expect("push");
    stack.push(U256::from(30u64)).expect("push");

    recorder.record_step(
        0x01, // ADD
        5,
        500_000,
        0,
        &stack,
        0,
        Address::zero(),
    );

    assert_eq!(recorder.steps.len(), 1);
    let step = &recorder.steps[0];
    // Should capture only 3 items (min(8, 3) = 3)
    assert_eq!(
        step.stack_top.len(),
        3,
        "should capture min(requested, available) items"
    );
    assert_eq!(step.stack_depth, 3);
    // Peek order: index 0 = top of stack = last pushed = 30
    assert_eq!(step.stack_top[0], U256::from(30u64));
    assert_eq!(step.stack_top[1], U256::from(20u64));
    assert_eq!(step.stack_top[2], U256::from(10u64));
}

#[test]
fn test_capture_stack_exact_match() {
    let config = ReplayConfig {
        stack_top_capture: 2, // request exactly 2
    };
    let mut recorder = DebugRecorder::new(config);
    let mut stack = Stack::default();

    stack.push(U256::from(100u64)).expect("push");
    stack.push(U256::from(200u64)).expect("push");

    recorder.record_step(0x01, 0, 1_000_000, 0, &stack, 0, Address::zero());

    let step = &recorder.steps[0];
    assert_eq!(step.stack_top.len(), 2);
    assert_eq!(step.stack_top[0], U256::from(200u64));
    assert_eq!(step.stack_top[1], U256::from(100u64));
}

#[test]
fn test_capture_stack_zero_config() {
    // Config requests 0 stack items — should always produce empty
    let config = ReplayConfig {
        stack_top_capture: 0,
    };
    let mut recorder = DebugRecorder::new(config);
    let mut stack = Stack::default();

    stack.push(U256::from(42u64)).expect("push");
    stack.push(U256::from(99u64)).expect("push");

    recorder.record_step(0x01, 0, 1_000_000, 0, &stack, 0, Address::zero());

    let step = &recorder.steps[0];
    assert!(
        step.stack_top.is_empty(),
        "stack_top_capture=0 should produce empty stack_top"
    );
    assert_eq!(
        step.stack_depth, 2,
        "stack_depth should still reflect actual depth"
    );
}

#[test]
fn test_capture_stack_more_items_than_requested() {
    let config = ReplayConfig {
        stack_top_capture: 2, // request only 2
    };
    let mut recorder = DebugRecorder::new(config);
    let mut stack = Stack::default();

    // Push 5 items
    for i in 1..=5u64 {
        stack.push(U256::from(i)).expect("push");
    }

    recorder.record_step(0x01, 0, 1_000_000, 0, &stack, 0, Address::zero());

    let step = &recorder.steps[0];
    assert_eq!(step.stack_top.len(), 2, "should only capture top 2");
    assert_eq!(step.stack_depth, 5, "stack_depth should show all 5 items");
    // Top of stack = last pushed = 5
    assert_eq!(step.stack_top[0], U256::from(5u64));
    assert_eq!(step.stack_top[1], U256::from(4u64));
}
