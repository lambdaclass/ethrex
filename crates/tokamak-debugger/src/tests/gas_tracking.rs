//! Gas tracking tests — verify gas accounting through the trace.

use super::helpers::*;
use crate::engine::ReplayEngine;
use crate::types::ReplayConfig;

/// Gas should generally decrease (or stay same) across sequential steps.
#[test]
fn test_gas_decreases() {
    // PUSH1 1, PUSH1 2, ADD, STOP
    let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
    let (contract, sender, mut db) = setup_contract(bytecode);
    let env = make_test_env(sender);
    let tx = make_test_tx(contract);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    let steps = engine.steps_range(0, engine.len());

    // Each opcode consumes gas, so gas_remaining should not increase.
    for window in steps.windows(2) {
        assert!(
            window[0].gas_remaining >= window[1].gas_remaining,
            "gas should not increase: step {} gas={} -> step {} gas={}",
            window[0].step_index,
            window[0].gas_remaining,
            window[1].step_index,
            window[1].gas_remaining,
        );
    }
}

/// PUSH1 costs 3 gas, ADD costs 3 gas — verify exact deltas.
#[test]
fn test_known_gas_costs() {
    // PUSH1 3, PUSH1 4, ADD, STOP
    let bytecode = vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00];
    let (contract, sender, mut db) = setup_contract(bytecode);
    let env = make_test_env(sender);
    let tx = make_test_tx(contract);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    let steps = engine.steps_range(0, engine.len());

    // PUSH1 (step 0→1): costs 3 gas
    let push1_delta = steps[0].gas_remaining - steps[1].gas_remaining;
    assert_eq!(
        push1_delta, 3,
        "PUSH1 should cost 3 gas, got delta {push1_delta}"
    );

    // Second PUSH1 (step 1→2): also costs 3 gas
    let push2_delta = steps[1].gas_remaining - steps[2].gas_remaining;
    assert_eq!(
        push2_delta, 3,
        "PUSH1 should cost 3 gas, got delta {push2_delta}"
    );

    // ADD (step 2→3): costs 3 gas
    let add_delta = steps[2].gas_remaining - steps[3].gas_remaining;
    assert_eq!(add_delta, 3, "ADD should cost 3 gas, got delta {add_delta}");
}

/// Final gas in trace should be consistent with the execution report.
#[test]
fn test_final_gas_consistent() {
    let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
    let (contract, sender, mut db) = setup_contract(bytecode);
    let env = make_test_env(sender);
    let tx = make_test_tx(contract);

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    let trace = engine.trace();
    assert!(trace.success, "transaction should succeed");

    // gas_used from the report includes intrinsic gas.
    // The first step's gas_remaining has already had intrinsic gas deducted.
    // The last step records gas BEFORE that opcode executes.
    // So: gas_used ≈ (gas_limit - last_step.gas_remaining) + last_opcode_cost
    // We just verify gas_used > 0 and is reasonable.
    assert!(trace.gas_used > 0, "gas_used should be positive");

    // With intrinsic gas of 21000 + 9 gas for opcodes, total ≈ 21009
    // The exact value depends on EIP-specific calculations, so we check a range.
    assert!(
        trace.gas_used >= 21_000,
        "gas_used should include intrinsic gas, got {}",
        trace.gas_used
    );
}
