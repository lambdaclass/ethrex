//! Integration tests for D-3 constant folding optimizer.
//!
//! Verifies that the optimizer is called during compilation and that
//! optimized bytecode produces correct execution results.

use bytes::Bytes;
use ethrex_common::U256;
use ethrex_common::types::Code;
use ethrex_levm::jit::optimizer;
use ethrex_levm::jit::types::AnalyzedBytecode;
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use rustc_hash::FxHashMap;

use crate::tests::test_helpers::*;

/// Test that the optimizer integrates with the backend analyze() path.
#[cfg(feature = "revmc-backend")]
#[test]
fn test_backend_analyze_applies_optimization() {
    use crate::backend::RevmcBackend;

    // PUSH1 3, PUSH1 4, ADD, STOP — should be folded to PUSH4 7, STOP
    let bytecode = vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00];
    let code = Code::from_bytecode(Bytes::from(bytecode));
    let backend = RevmcBackend::new();

    let analyzed = backend.analyze(&code).expect("analyze should succeed");

    // After optimization: PUSH4 7, STOP (2 opcodes instead of 4)
    assert_eq!(
        analyzed.opcode_count, 2,
        "should have 2 opcodes after folding"
    );
    assert_eq!(analyzed.bytecode[0], 0x63, "should be PUSH4 opcode");
    assert_eq!(analyzed.bytecode[4], 0x07, "should be folded result 7");
}

/// Test that optimization preserves execution correctness.
///
/// Bytecode: PUSH1 10, PUSH1 20, ADD, PUSH1 0, MSTORE, PUSH1 32, PUSH1 0, RETURN
/// Expected: returns 30 as a 32-byte big-endian word.
///
/// The PUSH1 10 + PUSH1 20 + ADD sequence should be folded to PUSH4 30,
/// but the RETURN output must still be 30.
#[test]
fn test_optimized_execution_correctness() {
    let bytecode = vec![
        0x60, 0x0A, // PUSH1 10
        0x60, 0x14, // PUSH1 20
        0x01, // ADD → 30
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE (store 30 at offset 0)
        0x60, 0x20, // PUSH1 32
        0x60, 0x00, // PUSH1 0
        0xf3, // RETURN (return 32 bytes from offset 0)
    ];
    let code = Code::from_bytecode(Bytes::from(bytecode));

    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());
    let mut db = make_test_db(accounts);
    let env = make_test_env(sender_addr);
    let tx = make_test_tx(contract_addr, Bytes::new());

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("VM::new should succeed");
    let report = vm.stateless_execute().expect("execution should succeed");

    assert!(report.is_success(), "should succeed");
    let result = U256::from_big_endian(&report.output);
    assert_eq!(result, U256::from(30), "10 + 20 = 30");
}

/// Test optimizer on bytecode with multiple foldable patterns.
///
/// Bytecode: PUSH1 3, PUSH1 4, ADD, PUSH1 5, PUSH1 6, MUL, ADD, PUSH1 0, MSTORE, ...
/// Expected: (3+4) + (5*6) = 7 + 30 = 37
#[test]
fn test_optimized_execution_multiple_folds() {
    let bytecode = vec![
        0x60, 0x03, // PUSH1 3
        0x60, 0x04, // PUSH1 4
        0x01, // ADD → 7
        0x60, 0x05, // PUSH1 5
        0x60, 0x06, // PUSH1 6
        0x02, // MUL → 30
        0x01, // ADD → 7 + 30 = 37
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE
        0x60, 0x20, // PUSH1 32
        0x60, 0x00, // PUSH1 0
        0xf3, // RETURN
    ];
    let code = Code::from_bytecode(Bytes::from(bytecode));

    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());
    let mut db = make_test_db(accounts);
    let env = make_test_env(sender_addr);
    let tx = make_test_tx(contract_addr, Bytes::new());

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("VM::new should succeed");
    let report = vm.stateless_execute().expect("execution should succeed");

    assert!(report.is_success(), "should succeed");
    let result = U256::from_big_endian(&report.output);
    assert_eq!(result, U256::from(37), "(3+4) + (5*6) = 37");
}

/// Test that optimizer correctly detects and folds patterns.
#[test]
fn test_optimizer_stats_on_foldable_bytecode() {
    // PUSH1 3, PUSH1 4, ADD, PUSH1 5, PUSH1 6, MUL, STOP
    let bytecode = Bytes::from(vec![
        0x60, 0x03, 0x60, 0x04, 0x01, 0x60, 0x05, 0x60, 0x06, 0x02, 0x00,
    ]);
    let analyzed = AnalyzedBytecode {
        hash: ethrex_common::H256::zero(),
        bytecode,
        jump_targets: vec![],
        basic_blocks: vec![],
        opcode_count: 7,
        has_external_calls: false,
    };

    let (optimized, stats) = optimizer::optimize(analyzed);

    assert_eq!(stats.patterns_detected, 2);
    assert_eq!(stats.patterns_folded, 2);
    assert_eq!(stats.opcodes_eliminated, 4);
    assert_eq!(optimized.opcode_count, 3); // 7 - 4
}

/// Test that unfoldable bytecode passes through unchanged.
#[test]
fn test_optimizer_no_patterns() {
    // PUSH1 3, DUP1, ADD, STOP — no PUSH+PUSH+OP pattern
    let bytecode = Bytes::from(vec![0x60, 0x03, 0x80, 0x01, 0x00]);
    let analyzed = AnalyzedBytecode {
        hash: ethrex_common::H256::zero(),
        bytecode: bytecode.clone(),
        jump_targets: vec![],
        basic_blocks: vec![],
        opcode_count: 4,
        has_external_calls: false,
    };

    let (optimized, stats) = optimizer::optimize(analyzed);

    assert_eq!(stats.patterns_detected, 0);
    assert_eq!(stats.patterns_folded, 0);
    assert_eq!(optimized.bytecode, bytecode);
}

/// Test that DIV/MOD constant folding executes correctly.
///
/// Bytecode: PUSH1 3, PUSH1 21, DIV → 7, then PUSH1 4, PUSH1 21, MOD → 1, ADD → 8
#[test]
fn test_optimized_execution_div_mod() {
    let bytecode = vec![
        0x60, 0x03, // PUSH1 3 (divisor)
        0x60, 0x15, // PUSH1 21 (dividend)
        0x04, // DIV → 7
        0x60, 0x04, // PUSH1 4 (divisor)
        0x60, 0x15, // PUSH1 21 (dividend)
        0x06, // MOD → 1
        0x01, // ADD → 7 + 1 = 8
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE
        0x60, 0x20, // PUSH1 32
        0x60, 0x00, // PUSH1 0
        0xf3, // RETURN
    ];
    let code = Code::from_bytecode(Bytes::from(bytecode));

    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());
    let mut db = make_test_db(accounts);
    let env = make_test_env(sender_addr);
    let tx = make_test_tx(contract_addr, Bytes::new());

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("VM::new should succeed");
    let report = vm.stateless_execute().expect("execution should succeed");

    assert!(report.is_success(), "should succeed");
    let result = U256::from_big_endian(&report.output);
    assert_eq!(result, U256::from(8), "21/3 + 21%4 = 7 + 1 = 8");
}

/// Test that comparison constant folding executes correctly.
///
/// Bytecode: PUSH1 5, PUSH1 3, LT → 1, PUSH1 3, PUSH1 3, EQ → 1, ADD → 2
#[test]
fn test_optimized_execution_comparison() {
    let bytecode = vec![
        0x60, 0x05, // PUSH1 5
        0x60, 0x03, // PUSH1 3
        0x10, // LT → 3 < 5 = 1
        0x60, 0x03, // PUSH1 3
        0x60, 0x03, // PUSH1 3
        0x14, // EQ → 3 == 3 = 1
        0x01, // ADD → 1 + 1 = 2
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE
        0x60, 0x20, // PUSH1 32
        0x60, 0x00, // PUSH1 0
        0xf3, // RETURN
    ];
    let code = Code::from_bytecode(Bytes::from(bytecode));

    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());
    let mut db = make_test_db(accounts);
    let env = make_test_env(sender_addr);
    let tx = make_test_tx(contract_addr, Bytes::new());

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("VM::new should succeed");
    let report = vm.stateless_execute().expect("execution should succeed");

    assert!(report.is_success(), "should succeed");
    let result = U256::from_big_endian(&report.output);
    assert_eq!(result, U256::from(2), "(3<5) + (3==3) = 1 + 1 = 2");
}

/// Test that shift constant folding executes correctly.
///
/// Bytecode: PUSH1 1, PUSH1 8, SHL → 256, PUSH1 8, SHR → 1 (round-trip)
/// The result of SHL is on stack, but to test with SHR we'd need a separate fold.
/// Instead: PUSH1 1, PUSH1 8, SHL → 256, store and return.
#[test]
fn test_optimized_execution_shift() {
    let bytecode = vec![
        0x60, 0x01, // PUSH1 1 (value)
        0x60, 0x08, // PUSH1 8 (shift amount)
        0x1B, // SHL → 1 << 8 = 256
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE
        0x60, 0x20, // PUSH1 32
        0x60, 0x00, // PUSH1 0
        0xf3, // RETURN
    ];
    let code = Code::from_bytecode(Bytes::from(bytecode));

    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());
    let mut db = make_test_db(accounts);
    let env = make_test_env(sender_addr);
    let tx = make_test_tx(contract_addr, Bytes::new());

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("VM::new should succeed");
    let report = vm.stateless_execute().expect("execution should succeed");

    assert!(report.is_success(), "should succeed");
    let result = U256::from_big_endian(&report.output);
    assert_eq!(result, U256::from(256), "1 << 8 = 256");
}

/// Test that bitwise constant folding executes correctly.
///
/// Bytecode: PUSH1 0xFF, PUSH1 0x0F, AND → 0x0F
#[test]
fn test_optimized_execution_bitwise() {
    let bytecode = vec![
        0x60, 0xFF, // PUSH1 0xFF
        0x60, 0x0F, // PUSH1 0x0F
        0x16, // AND → 0x0F
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE
        0x60, 0x20, // PUSH1 32
        0x60, 0x00, // PUSH1 0
        0xf3, // RETURN
    ];
    let code = Code::from_bytecode(Bytes::from(bytecode));

    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());
    let mut db = make_test_db(accounts);
    let env = make_test_env(sender_addr);
    let tx = make_test_tx(contract_addr, Bytes::new());

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("VM::new should succeed");
    let report = vm.stateless_execute().expect("execution should succeed");

    assert!(report.is_success(), "should succeed");
    let result = U256::from_big_endian(&report.output);
    assert_eq!(result, U256::from(0x0F), "0xFF AND 0x0F = 0x0F");
}
