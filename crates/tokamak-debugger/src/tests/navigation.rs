//! Navigation tests — forward/backward/goto cursor operations.

use super::helpers::*;
use crate::engine::ReplayEngine;
use crate::types::ReplayConfig;

/// Helper: create a small replay engine with `PUSH1 1, PUSH1 2, ADD, STOP` (4 steps).
fn make_4step_engine() -> ReplayEngine {
    let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
    let (contract, sender, mut db) = setup_contract(bytecode);
    let env = make_test_env(sender);
    let tx = make_test_tx(contract);

    ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default()).expect("record should succeed")
}

#[test]
fn test_forward_backward_cursor() {
    let mut engine = make_4step_engine();

    assert_eq!(engine.position(), 0);
    assert_eq!(engine.current_step().unwrap().opcode, 0x60); // PUSH1

    // Forward 3 times
    let step1 = engine.forward().unwrap();
    assert_eq!(step1.step_index, 1);
    assert_eq!(engine.position(), 1);

    let step2 = engine.forward().unwrap();
    assert_eq!(step2.step_index, 2);

    let step3 = engine.forward().unwrap();
    assert_eq!(step3.step_index, 3);

    // Backward once
    let step2_back = engine.backward().unwrap();
    assert_eq!(step2_back.step_index, 2);
    assert_eq!(engine.position(), 2);
}

#[test]
fn test_goto_first_middle_last() {
    let mut engine = make_4step_engine();

    // Go to last
    let last = engine.goto(3).unwrap();
    assert_eq!(last.step_index, 3);
    assert_eq!(last.opcode, 0x00); // STOP

    // Go to middle
    let mid = engine.goto(1).unwrap();
    assert_eq!(mid.step_index, 1);

    // Go to first
    let first = engine.goto(0).unwrap();
    assert_eq!(first.step_index, 0);
    assert_eq!(first.pc, 0);
}

#[test]
fn test_goto_out_of_bounds_returns_none() {
    let mut engine = make_4step_engine();

    assert!(engine.goto(4).is_none());
    assert!(engine.goto(100).is_none());
    // Cursor should not have moved
    assert_eq!(engine.position(), 0);
}

#[test]
fn test_backward_at_zero_returns_none() {
    let mut engine = make_4step_engine();

    assert_eq!(engine.position(), 0);
    assert!(engine.backward().is_none());
    assert_eq!(engine.position(), 0);
}

#[test]
fn test_forward_at_end_returns_none() {
    let mut engine = make_4step_engine();

    // Move to last step
    engine.goto(3);
    assert_eq!(engine.position(), 3);

    assert!(engine.forward().is_none());
    assert_eq!(engine.position(), 3);
}
