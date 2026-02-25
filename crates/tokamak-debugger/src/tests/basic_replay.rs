//! Basic replay tests — verify step recording and opcode/PC values.

use super::helpers::*;
use crate::engine::ReplayEngine;
use crate::types::ReplayConfig;
use ethrex_common::U256;

/// PUSH1 3, PUSH1 4, ADD, STOP → 4 steps with correct opcodes and PCs.
#[test]
fn test_push_add_stop_trace() {
    // Bytecode: PUSH1 3, PUSH1 4, ADD, STOP
    // Opcodes:  0x60 0x03, 0x60 0x04, 0x01, 0x00
    let bytecode = vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00];
    let (contract, sender, mut db) = setup_contract(bytecode);
    let env = make_test_env(sender);
    let tx = make_test_tx(contract);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    assert_eq!(
        engine.len(),
        4,
        "expected 4 steps (PUSH1, PUSH1, ADD, STOP)"
    );

    let steps = engine.steps_range(0, 4);

    // Step 0: PUSH1 at PC 0
    assert_eq!(steps[0].opcode, 0x60);
    assert_eq!(steps[0].pc, 0);

    // Step 1: PUSH1 at PC 2
    assert_eq!(steps[1].opcode, 0x60);
    assert_eq!(steps[1].pc, 2);

    // Step 2: ADD at PC 4
    assert_eq!(steps[2].opcode, 0x01);
    assert_eq!(steps[2].pc, 4);

    // Step 3: STOP at PC 5
    assert_eq!(steps[3].opcode, 0x00);
    assert_eq!(steps[3].pc, 5);
}

/// Verify step count matches number of executed opcodes.
#[test]
fn test_step_count_matches() {
    // 10x PUSH1 + POP pairs (20 opcodes) + STOP (1)
    let mut bytecode = Vec::new();
    for i in 0..10u8 {
        bytecode.push(0x60); // PUSH1
        bytecode.push(i);
        bytecode.push(0x50); // POP
    }
    bytecode.push(0x00); // STOP

    let (contract, sender, mut db) = setup_contract(bytecode);
    let env = make_test_env(sender);
    let tx = make_test_tx(contract);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    // 10 PUSH1 + 10 POP + 1 STOP = 21
    assert_eq!(engine.len(), 21);
}

/// After PUSH1 5, stack_top[0] should be 5.
#[test]
fn test_stack_top_captured() {
    // PUSH1 5, STOP
    let bytecode = vec![0x60, 0x05, 0x00];
    let (contract, sender, mut db) = setup_contract(bytecode);
    let env = make_test_env(sender);
    let tx = make_test_tx(contract);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    assert_eq!(engine.len(), 2, "PUSH1 + STOP");

    // At step 1 (STOP), the stack should contain the pushed value.
    // We record state BEFORE execution, so step 1 sees the post-PUSH1 state.
    let stop_step = &engine.trace().steps[1];
    assert_eq!(stop_step.stack_depth, 1);
    assert_eq!(stop_step.stack_top[0], U256::from(5u64));
}

/// STOP-only bytecode → exactly 1 step.
#[test]
fn test_empty_stop() {
    let bytecode = vec![0x00]; // STOP
    let (contract, sender, mut db) = setup_contract(bytecode);
    let env = make_test_env(sender);
    let tx = make_test_tx(contract);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    assert_eq!(engine.len(), 1);
    assert_eq!(engine.trace().steps[0].opcode, 0x00);
}
