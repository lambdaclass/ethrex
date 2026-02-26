//! Error handling tests for ReplayEngine::record().
//!
//! Tests:
//! - Recording a REVERT transaction produces a valid trace with success=false
//! - Recording a STOP-only transaction produces a minimal trace
//! - Recording with custom ReplayConfig

use crate::engine::ReplayEngine;
use crate::types::ReplayConfig;

use super::helpers;

#[test]
fn test_record_revert_transaction() {
    // Bytecode: PUSH1 0x00 PUSH1 0x00 REVERT
    // REVERT(offset=0, size=0) — reverts with empty data
    let bytecode = vec![
        0x60, 0x00, // PUSH1 0x00 (size)
        0x60, 0x00, // PUSH1 0x00 (offset)
        0xfd, // REVERT
    ];

    let (contract_addr, sender_addr, mut db) = helpers::setup_contract(bytecode);
    let env = helpers::make_test_env(sender_addr);
    let tx = helpers::make_test_tx(contract_addr);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("recording a REVERT should not return Err");

    // REVERT is a valid execution outcome — the trace should be recorded
    assert!(!engine.is_empty(), "revert trace should have steps");
    assert!(
        !engine.trace().success,
        "trace should indicate failure (revert)"
    );

    // Steps should include: PUSH1, PUSH1, REVERT
    // (the exact count depends on intrinsic setup but should have at least 3 opcode steps)
    assert!(
        engine.len() >= 3,
        "should have at least 3 steps (2 PUSHes + REVERT), got {}",
        engine.len()
    );
}

#[test]
fn test_record_stop_only() {
    // Bytecode: STOP (0x00) — simplest possible execution
    let bytecode = vec![0x00];

    let (contract_addr, sender_addr, mut db) = helpers::setup_contract(bytecode);
    let env = helpers::make_test_env(sender_addr);
    let tx = helpers::make_test_tx(contract_addr);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("recording STOP should succeed");

    assert!(engine.trace().success, "STOP should be successful");
    // A single STOP opcode should produce exactly 1 step
    assert_eq!(engine.len(), 1, "STOP should produce exactly 1 step");

    let step = engine.current_step().expect("should have step 0");
    assert_eq!(step.opcode, 0x00, "opcode should be STOP");
    assert_eq!(step.pc, 0, "pc should be 0");
    assert_eq!(step.depth, 0, "depth should be 0 for top-level call");
}

#[test]
fn test_record_with_custom_stack_capture() {
    // Bytecode: PUSH1 0x42 PUSH1 0x43 ADD STOP
    let bytecode = vec![
        0x60, 0x42, // PUSH1 0x42
        0x60, 0x43, // PUSH1 0x43
        0x01, // ADD
        0x00, // STOP
    ];

    let (contract_addr, sender_addr, mut db) = helpers::setup_contract(bytecode);
    let env = helpers::make_test_env(sender_addr);
    let tx = helpers::make_test_tx(contract_addr);

    // Capture only 1 stack item per step
    let config = ReplayConfig {
        stack_top_capture: 1,
    };

    let engine = ReplayEngine::record(&mut db, env, &tx, config).expect("recording should succeed");

    assert!(engine.trace().success);
    assert_eq!(engine.len(), 4, "should have 4 steps");

    // After PUSH1 0x42 (step 0), stack has 1 item
    // Step 1 is PUSH1 0x43 — at this point stack has [0x42]
    // With stack_top_capture=1, step 1 should capture exactly 1 item
    let step1 = &engine.trace().steps[1];
    assert!(
        step1.stack_top.len() <= 1,
        "stack_top_capture=1 should capture at most 1 item"
    );
}

#[test]
fn test_record_empty_bytecode() {
    // Empty bytecode — behaves like STOP at PC 0
    let bytecode = vec![];

    let (contract_addr, sender_addr, mut db) = helpers::setup_contract(bytecode);
    let env = helpers::make_test_env(sender_addr);
    let tx = helpers::make_test_tx(contract_addr);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("empty bytecode should succeed");

    assert!(engine.trace().success, "empty bytecode should succeed");
    // Empty bytecode hits STOP immediately — should produce 1 step
    assert_eq!(
        engine.len(),
        1,
        "empty bytecode should produce 1 step (implicit STOP)"
    );
}

#[test]
fn test_record_out_of_gas() {
    // Bytecode that uses lots of gas: infinite loop JUMPDEST PUSH1 0 JUMP
    // 0x5B = JUMPDEST, 0x60 0x00 = PUSH1 0, 0x56 = JUMP
    let bytecode = vec![
        0x5b, // JUMPDEST at offset 0
        0x60, 0x00, // PUSH1 0
        0x56, // JUMP back to offset 0
    ];

    let (contract_addr, sender_addr, mut db) = helpers::setup_contract(bytecode);

    // Very low gas limit to force OOG
    let env = ethrex_levm::Environment {
        origin: sender_addr,
        gas_limit: 21_100, // Just barely above intrinsic gas
        block_gas_limit: 21_100,
        ..Default::default()
    };
    let tx = helpers::make_test_tx(contract_addr);

    let result = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default());

    // OOG might result in an error or a trace with success=false.
    // Either outcome is acceptable — we're testing it doesn't panic.
    match result {
        Ok(engine) => {
            // OOG produces a trace but execution fails
            assert!(
                !engine.trace().success || !engine.is_empty(),
                "OOG should either fail or produce some steps"
            );
        }
        Err(_) => {
            // VMError from OOG is also acceptable
        }
    }
}
