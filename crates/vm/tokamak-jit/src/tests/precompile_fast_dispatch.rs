//! G-8: Precompile fast dispatch tests.
//!
//! Validates that when JIT-compiled code calls a precompile (ECADD, SHA256,
//! IDENTITY), the precompile executes correctly and fast dispatch metrics
//! are tracked.
#![allow(clippy::vec_init_then_push)]
#![cfg_attr(not(feature = "revmc-backend"), allow(dead_code))]

use crate::tests::test_helpers::*;
use bytes::Bytes;
use ethrex_common::types::Code;
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use rustc_hash::FxHashMap;

/// Build a contract that CALLs the IDENTITY precompile (0x04) with 32 bytes of calldata.
///
/// Stores 42 at mem[0..32], STATICCALL identity at mem[32..64], returns output.
fn make_identity_precompile_caller() -> Vec<u8> {
    let mut code = Vec::new();

    // PUSH32 <test input: 42 as 32 bytes>
    code.push(0x7F); // PUSH32
    let mut input = [0u8; 32];
    input[31] = 42;
    code.extend_from_slice(&input);
    // PUSH1 0x00 / MSTORE
    code.push(0x60);
    code.push(0x00);
    code.push(0x52);

    // STATICCALL to identity precompile (0x04)
    code.push(0x60);
    code.push(0x20); // retSize = 32
    code.push(0x60);
    code.push(0x20); // retOffset = 32
    code.push(0x60);
    code.push(0x20); // argsSize = 32
    code.push(0x60);
    code.push(0x00); // argsOffset = 0
    code.push(0x73); // PUSH20
    let mut addr = [0u8; 20];
    addr[19] = 0x04;
    code.extend_from_slice(&addr);
    code.push(0x62); // PUSH3 gas
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFA); // STATICCALL
    code.push(0x50); // POP

    // RETURN mem[32..64]
    code.push(0x60);
    code.push(0x20);
    code.push(0x60);
    code.push(0x20);
    code.push(0xF3);

    code
}

/// Build a contract that CALLs the SHA256 precompile (0x02) with 32 bytes.
fn make_sha256_precompile_caller() -> Vec<u8> {
    let mut code = Vec::new();

    // PUSH32 <input: 1>
    code.push(0x7F);
    let mut input = [0u8; 32];
    input[31] = 1;
    code.extend_from_slice(&input);
    code.push(0x60);
    code.push(0x00);
    code.push(0x52);

    // STATICCALL to SHA256 precompile (0x02)
    code.push(0x60);
    code.push(0x20); // retSize
    code.push(0x60);
    code.push(0x20); // retOffset
    code.push(0x60);
    code.push(0x20); // argsSize
    code.push(0x60);
    code.push(0x00); // argsOffset
    code.push(0x73); // PUSH20
    let mut addr = [0u8; 20];
    addr[19] = 0x02;
    code.extend_from_slice(&addr);
    code.push(0x62); // PUSH3 gas
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFA); // STATICCALL
    code.push(0x50); // POP

    code.push(0x60);
    code.push(0x20);
    code.push(0x60);
    code.push(0x20);
    code.push(0xF3);

    code
}

/// Build a contract that CALLs the ECADD precompile (0x06) with two zero points.
/// ECADD(0,0, 0,0) = (0,0) — simplest valid input.
/// Input is 128 bytes of zeros (clean memory), output is 64 bytes.
fn make_ecadd_precompile_caller() -> Vec<u8> {
    let mut code = Vec::new();

    // Input is 128 bytes of zeros (already in clean memory)
    code.push(0x60);
    code.push(0x40); // retSize = 64
    code.push(0x61);
    code.push(0x00);
    code.push(0x80); // retOffset = 128
    code.push(0x61);
    code.push(0x00);
    code.push(0x80); // argsSize = 128
    code.push(0x60);
    code.push(0x00); // argsOffset = 0
    code.push(0x73); // PUSH20
    let mut addr = [0u8; 20];
    addr[19] = 0x06;
    code.extend_from_slice(&addr);
    code.push(0x62); // PUSH3 gas
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFA); // STATICCALL
    code.push(0x50); // POP

    // RETURN output (64 bytes at offset 128)
    code.push(0x60);
    code.push(0x40);
    code.push(0x61);
    code.push(0x00);
    code.push(0x80);
    code.push(0xF3);

    code
}

/// Build a contract that calls a precompile N times in a loop.
fn make_precompile_loop_caller(precompile_addr: u8, iterations: u8) -> Vec<u8> {
    let mut code = Vec::new();

    // Store data at mem[0..32] for the precompile
    code.push(0x7F); // PUSH32
    let mut input = [0u8; 32];
    input[31] = 0xFF;
    code.extend_from_slice(&input);
    code.push(0x60);
    code.push(0x00);
    code.push(0x52);

    // PUSH1 0 (i = 0)
    code.push(0x60);
    code.push(0x00);

    let loop_start = code.len();
    code.push(0x5B); // JUMPDEST
    code.push(0x80); // DUP1
    code.push(0x60); // PUSH1 iterations
    code.push(iterations);
    code.push(0x10); // LT
    code.push(0x15); // ISZERO
    code.push(0x60); // PUSH1 <exit_dest>
    let exit_patch = code.len();
    code.push(0x00); // placeholder
    code.push(0x57); // JUMPI

    // STATICCALL to precompile
    code.push(0x60);
    code.push(0x20); // retSize
    code.push(0x60);
    code.push(0x20); // retOffset
    code.push(0x60);
    code.push(0x20); // argsSize
    code.push(0x60);
    code.push(0x00); // argsOffset
    code.push(0x73); // PUSH20
    let mut addr = [0u8; 20];
    addr[19] = precompile_addr;
    code.extend_from_slice(&addr);
    code.push(0x62); // PUSH3 gas
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFA); // STATICCALL
    code.push(0x50); // POP

    // i++
    code.push(0x60);
    code.push(0x01);
    code.push(0x01); // ADD

    // JUMP loop_start
    code.push(0x60);
    #[expect(clippy::as_conversions)]
    {
        code.push(loop_start as u8);
    }
    code.push(0x56); // JUMP

    // exit:
    let exit_dest = code.len();
    code.push(0x5B); // JUMPDEST
    code.push(0x50); // POP i
    code.push(0x00); // STOP

    #[expect(clippy::as_conversions)]
    {
        code[exit_patch] = exit_dest as u8;
    }

    code
}

/// Helper: run bytecode via interpreter and return (success, output, gas_used).
fn run_interpreter(bytecode: Vec<u8>) -> (bool, Vec<u8>, u64) {
    let code = Code::from_bytecode(Bytes::from(bytecode));
    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());
    let mut db = make_test_db(accounts);
    let env = make_test_env(sender_addr);
    let tx = make_test_tx(contract_addr, Bytes::new());

    let mut vm =
        VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).expect("VM creation");

    let report = vm.execute().expect("execution");
    (report.is_success(), report.output.to_vec(), report.gas_used)
}

// ===== Interpreter correctness tests =====

/// Identity precompile CALL returns correct output.
#[test]
fn test_g8_identity_precompile_correct_output() {
    let bytecode = make_identity_precompile_caller();
    let (success, output, _gas) = run_interpreter(bytecode);
    assert!(success, "identity precompile call should succeed");
    assert_eq!(output.len(), 32);
    assert_eq!(output[31], 42, "identity should return input unchanged");
}

/// SHA256 precompile CALL returns correct 32-byte hash.
#[test]
fn test_g8_sha256_precompile_correct_output() {
    let bytecode = make_sha256_precompile_caller();
    let (success, output, _gas) = run_interpreter(bytecode);
    assert!(success, "sha256 precompile call should succeed");
    assert_eq!(output.len(), 32);
    // SHA256 of 32-byte input [0..0, 1] should be non-zero
    assert!(
        output.iter().any(|&b| b != 0),
        "sha256 output should be non-zero"
    );
}

/// ECADD precompile with zero points returns zero.
#[test]
fn test_g8_ecadd_zero_points_returns_zero() {
    let bytecode = make_ecadd_precompile_caller();
    let (success, output, _gas) = run_interpreter(bytecode);
    assert!(success, "ecadd precompile call should succeed");
    assert_eq!(output.len(), 64);
    assert!(
        output.iter().all(|&b| b == 0),
        "ecadd(0,0 + 0,0) should return zero"
    );
}

/// Multiple IDENTITY calls in a loop.
#[test]
fn test_g8_precompile_loop_identity() {
    let bytecode = make_precompile_loop_caller(0x04, 10);
    let (success, _output, _gas) = run_interpreter(bytecode);
    assert!(success, "10 identity calls in a loop should succeed");
}

/// Multiple SHA256 calls in a loop.
#[test]
fn test_g8_precompile_loop_sha256() {
    let bytecode = make_precompile_loop_caller(0x02, 5);
    let (success, _output, _gas) = run_interpreter(bytecode);
    assert!(success, "5 sha256 calls in a loop should succeed");
}

// ===== JIT differential tests (require revmc-backend) =====

/// JIT execution of precompile CALL matches interpreter result.
#[cfg(feature = "revmc-backend")]
#[test]
fn test_g8_precompile_jit_matches_interpreter() {
    use ethrex_levm::jit::dispatch::JIT_STATE;

    JIT_STATE.reset_for_testing();

    let bytecode = make_identity_precompile_caller();
    let (interp_ok, interp_output, _interp_gas) = run_interpreter(bytecode.clone());

    let code = Code::from_bytecode(Bytes::from(bytecode));
    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());

    // Run threshold+1 times to trigger compilation then JIT execution
    let threshold = JIT_STATE.config.compilation_threshold as usize;
    for run in 0..threshold + 1 {
        let mut db = make_test_db(accounts.clone());
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation");

        let report = vm.execute().expect("execution");

        if run == threshold {
            assert_eq!(
                report.is_success(),
                interp_ok,
                "JIT success should match interpreter"
            );
            assert_eq!(
                report.output.to_vec(),
                interp_output,
                "JIT output should match interpreter output"
            );
        }
    }
}

/// Precompile fast dispatch metric is tracked when JIT calls a precompile.
#[cfg(feature = "revmc-backend")]
#[test]
fn test_g8_precompile_fast_dispatch_metric_tracked() {
    use ethrex_levm::jit::dispatch::JIT_STATE;
    use std::sync::atomic::Ordering;

    JIT_STATE.reset_for_testing();

    let bytecode = make_identity_precompile_caller();
    let code = Code::from_bytecode(Bytes::from(bytecode));
    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());

    let threshold = JIT_STATE.config.compilation_threshold as usize;

    // Run threshold+1 times to compile + execute via JIT
    for _ in 0..threshold + 1 {
        let mut db = make_test_db(accounts.clone());
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation");
        vm.execute().expect("execution");
    }

    let fast_dispatches = JIT_STATE
        .metrics
        .precompile_fast_dispatches
        .load(Ordering::Relaxed);
    // At least 1 precompile fast dispatch should occur during JIT execution
    assert!(
        fast_dispatches >= 1,
        "precompile_fast_dispatches should be >= 1, got {fast_dispatches}"
    );
}

/// JIT differential: precompile loop matches interpreter gas within tolerance.
#[cfg(feature = "revmc-backend")]
#[test]
fn test_g8_precompile_loop_jit_differential() {
    use ethrex_levm::jit::dispatch::JIT_STATE;

    JIT_STATE.reset_for_testing();

    let bytecode = make_precompile_loop_caller(0x04, 5);
    let (interp_ok, _interp_output, interp_gas) = run_interpreter(bytecode.clone());

    let code = Code::from_bytecode(Bytes::from(bytecode));
    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());

    let threshold = JIT_STATE.config.compilation_threshold as usize;
    let mut jit_gas = 0u64;
    for run in 0..threshold + 2 {
        let mut db = make_test_db(accounts.clone());
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation");

        let report = vm.execute().expect("execution");

        if run == threshold + 1 {
            assert_eq!(report.is_success(), interp_ok);
            jit_gas = report.gas_used;
        }
    }

    // Gas should be within 10% tolerance
    let diff = jit_gas.abs_diff(interp_gas);
    let tolerance = interp_gas / 10;
    assert!(
        diff <= tolerance,
        "gas difference ({diff}) should be within 10% tolerance ({tolerance})"
    );
}

/// SHA256 precompile JIT differential.
#[cfg(feature = "revmc-backend")]
#[test]
fn test_g8_sha256_jit_differential() {
    use ethrex_levm::jit::dispatch::JIT_STATE;

    JIT_STATE.reset_for_testing();

    let bytecode = make_sha256_precompile_caller();
    let (interp_ok, interp_output, _) = run_interpreter(bytecode.clone());

    let code = Code::from_bytecode(Bytes::from(bytecode));
    let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, FxHashMap::default());

    let threshold = JIT_STATE.config.compilation_threshold as usize;
    for run in 0..threshold + 1 {
        let mut db = make_test_db(accounts.clone());
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation");

        let report = vm.execute().expect("execution");

        if run == threshold {
            assert_eq!(report.is_success(), interp_ok);
            assert_eq!(
                report.output.to_vec(),
                interp_output,
                "SHA256 JIT output mismatch"
            );
        }
    }
}
