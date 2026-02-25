//! SLOAD/SSTORE E2E tests for the JIT compiler.
//!
//! Tests:
//! - Counter contract: SLOAD→ADD→SSTORE→RETURN (JIT vs interpreter)
//! - SSTORE→REVERT rollback: verifies storage journal restores original values
//! - Multi-SSTORE→REVERT: verifies journal ordering for same-slot overwrites
//! - Gas comparison: JIT gas_used matches interpreter gas_used
#![allow(clippy::vec_init_then_push)]

use bytes::Bytes;
use ethrex_common::H256;

/// Build counter contract bytecode:
///
/// ```text
/// PUSH1 0x00  SLOAD         // load slot 0
/// PUSH1 0x01  ADD           // add 1
/// DUP1                      // dup for SSTORE and RETURN
/// PUSH1 0x00  SSTORE        // store back to slot 0
/// PUSH1 0x00  MSTORE        // store result in memory
/// PUSH1 0x20  PUSH1 0x00  RETURN
/// ```
///
/// Pre-seed slot 0 with 5 → result should be 6.
pub fn make_counter_bytecode() -> Vec<u8> {
    let mut code = Vec::new();

    code.push(0x60);
    code.push(0x00); //  0: PUSH1 0x00
    code.push(0x54); //  2: SLOAD        → [slot0_value]
    code.push(0x60);
    code.push(0x01); //  3: PUSH1 0x01
    code.push(0x01); //  5: ADD          → [slot0_value + 1]
    code.push(0x80); //  6: DUP1         → [val, val]
    code.push(0x60);
    code.push(0x00); //  7: PUSH1 0x00
    code.push(0x55); //  9: SSTORE       → [val]  (store val at slot 0)
    code.push(0x60);
    code.push(0x00); // 10: PUSH1 0x00
    code.push(0x52); // 12: MSTORE       → []     (mem[0..32] = val)
    code.push(0x60);
    code.push(0x20); // 13: PUSH1 0x20
    code.push(0x60);
    code.push(0x00); // 15: PUSH1 0x00
    code.push(0xf3); // 17: RETURN

    code
}

#[cfg(test)]
mod tests {
    use super::*;

    use ethrex_common::types::Code;
    use ethrex_common::{Address, U256};
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{VM, VMType};
    use rustc_hash::FxHashMap;

    use crate::tests::test_helpers::{
        INTRINSIC_GAS, TEST_GAS_LIMIT, TestAccount, make_contract_accounts, make_test_db,
        make_test_env, make_test_tx,
    };

    #[test]
    fn test_counter_bytecode_is_valid() {
        let code = make_counter_bytecode();
        assert!(!code.is_empty());
        assert!(code.contains(&0x54), "should contain SLOAD");
        assert!(code.contains(&0x55), "should contain SSTORE");
        assert_eq!(code.last(), Some(&0xf3), "should end with RETURN");
    }

    /// Run the counter contract through the LEVM interpreter.
    ///
    /// Pre-seeds storage slot 0 with value 5, expects output = 6.
    #[test]
    fn test_counter_interpreter_execution() {
        let bytecode = Bytes::from(make_counter_bytecode());
        let counter_code = Code::from_bytecode(bytecode);

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let (contract_addr, sender_addr, accounts) = make_contract_accounts(counter_code, storage);
        let mut db = make_test_db(accounts);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("counter execution should succeed");

        assert!(
            report.is_success(),
            "counter should succeed, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes");
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(result_val, U256::from(6u64), "5 + 1 = 6");
    }

    /// Compile the counter contract via revmc/LLVM JIT and validate output
    /// matches the interpreter path.
    ///
    /// This exercises SLOAD/SSTORE through the JIT host, validating
    /// EIP-2929 cold/warm tracking (Fix 4) and storage correctness.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_counter_jit_vs_interpreter() {
        use ethrex_levm::{jit::cache::CodeCache, vm::JIT_STATE};

        use crate::backend::RevmcBackend;
        use crate::execution::execute_jit;

        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        let fork = ethrex_common::types::Fork::Cancun;

        let bytecode = Bytes::from(make_counter_bytecode());
        let counter_code = Code::from_bytecode(bytecode);

        // Compile the bytecode via JIT
        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        backend
            .compile_and_cache(&counter_code, fork, &code_cache)
            .expect("JIT compilation should succeed");
        let compiled = code_cache
            .get(&(counter_code.hash, fork))
            .expect("compiled code should be in cache");

        // Pre-seed storage: slot 0 = 5
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        // --- Interpreter path ---
        let (contract_addr, sender_addr, interp_accounts) =
            make_contract_accounts(counter_code.clone(), storage.clone());
        let mut interp_db = make_test_db(interp_accounts);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(
            env.clone(),
            &mut interp_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("Interpreter VM::new should succeed");

        let interp_report = vm
            .stateless_execute()
            .expect("Interpreter counter execution should succeed");

        assert!(
            interp_report.is_success(),
            "Interpreter counter should succeed, got: {:?}",
            interp_report.result
        );
        let interp_result = U256::from_big_endian(&interp_report.output);
        assert_eq!(interp_result, U256::from(6u64), "Interpreter: 5 + 1 = 6");

        // --- JIT direct execution path ---
        let (_, _, jit_accounts) = make_contract_accounts(counter_code.clone(), storage);
        let mut jit_db = make_test_db(jit_accounts);

        #[expect(clippy::as_conversions)]
        let mut call_frame = ethrex_levm::call_frame::CallFrame::new(
            sender_addr,
            contract_addr,
            contract_addr,
            counter_code,
            U256::zero(),
            Bytes::new(),
            false,
            TEST_GAS_LIMIT,
            0,
            false,
            false,
            0,
            0,
            ethrex_levm::call_frame::Stack::default(),
            ethrex_levm::memory::Memory::default(),
        );

        let mut substate = ethrex_levm::vm::Substate::default();
        let mut storage_original_values = FxHashMap::default();

        let jit_outcome = execute_jit(
            &compiled,
            &mut call_frame,
            &mut jit_db,
            &mut substate,
            &env,
            &mut storage_original_values,
        )
        .expect("JIT counter execution should succeed");

        // Compare results
        match jit_outcome {
            ethrex_levm::jit::types::JitOutcome::Success { output, .. } => {
                assert_eq!(
                    output, interp_report.output,
                    "JIT and interpreter output mismatch"
                );
                let jit_result = U256::from_big_endian(&output);
                assert_eq!(jit_result, U256::from(6u64), "JIT: 5 + 1 = 6");

                // Note: JitOutcome::gas_used is execution-only gas (excludes intrinsic).
                // The interpreter's gas_used includes INTRINSIC_GAS for basic tx.
                // The corrected apply_jit_outcome formula computes gas from call_frame
                // (gas_limit - gas_remaining), which matches the interpreter. We verify
                // this separately in test_jit_gas_matches_interpreter.
            }
            other => {
                panic!("Expected JIT success, got: {other:?}");
            }
        }
    }

    /// Build bytecode that SSTOREs a value then REVERTs.
    ///
    /// ```text
    /// PUSH1 0x42  PUSH1 0x00  SSTORE   // slot 0 = 0x42
    /// PUSH1 0x00  PUSH1 0x00  REVERT   // revert with empty data
    /// ```
    ///
    /// Pre-seed slot 0 with 5 → after REVERT, slot 0 should still be 5.
    #[cfg(feature = "revmc-backend")]
    fn make_sstore_revert_bytecode() -> Vec<u8> {
        let mut code = Vec::new();
        code.push(0x60);
        code.push(0x42); // PUSH1 0x42
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0x55); // SSTORE (slot 0 = 0x42)
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0xfd); // REVERT
        code
    }

    /// Build bytecode that SSTOREs the same slot multiple times then REVERTs.
    ///
    /// ```text
    /// PUSH1 0x0A  PUSH1 0x00  SSTORE   // slot 0 = 10
    /// PUSH1 0x14  PUSH1 0x00  SSTORE   // slot 0 = 20
    /// PUSH1 0x1E  PUSH1 0x00  SSTORE   // slot 0 = 30
    /// PUSH1 0x00  PUSH1 0x00  REVERT
    /// ```
    ///
    /// Pre-seed slot 0 with 5 → after REVERT, slot 0 should still be 5.
    #[cfg(feature = "revmc-backend")]
    fn make_multi_sstore_revert_bytecode() -> Vec<u8> {
        let mut code = Vec::new();
        code.push(0x60);
        code.push(0x0A); // PUSH1 10
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0x55); // SSTORE (slot 0 = 10)
        code.push(0x60);
        code.push(0x14); // PUSH1 20
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0x55); // SSTORE (slot 0 = 20)
        code.push(0x60);
        code.push(0x1E); // PUSH1 30
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0x55); // SSTORE (slot 0 = 30)
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0xfd); // REVERT
        code
    }

    /// Build bytecode that SSTOREs to two different slots then REVERTs.
    ///
    /// ```text
    /// PUSH1 0x0A  PUSH1 0x00  SSTORE   // slot 0 = 10
    /// PUSH1 0x14  PUSH1 0x01  SSTORE   // slot 1 = 20
    /// PUSH1 0x00  PUSH1 0x00  REVERT
    /// ```
    ///
    /// Pre-seed slot 0 = 5, slot 1 = 7 → after REVERT, both should be restored.
    #[cfg(feature = "revmc-backend")]
    fn make_two_slot_sstore_revert_bytecode() -> Vec<u8> {
        let mut code = Vec::new();
        code.push(0x60);
        code.push(0x0A); // PUSH1 10
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0x55); // SSTORE (slot 0 = 10)
        code.push(0x60);
        code.push(0x14); // PUSH1 20
        code.push(0x60);
        code.push(0x01); // PUSH1 0x01
        code.push(0x55); // SSTORE (slot 1 = 20)
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0x60);
        code.push(0x00); // PUSH1 0x00
        code.push(0xfd); // REVERT
        code
    }

    /// Verify that JIT SSTORE→REVERT correctly rolls back storage.
    ///
    /// Pre-seeds slot 0 = 5, runs bytecode that writes slot 0 = 0x42 then REVERTs.
    /// After JIT execution, slot 0 must still be 5 (journal rollback applied).
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_sstore_revert_rollback() {
        use ethrex_levm::{jit::cache::CodeCache, vm::JIT_STATE};

        use crate::backend::RevmcBackend;
        use crate::execution::execute_jit;

        JIT_STATE.reset_for_testing();

        let fork = ethrex_common::types::Fork::Cancun;

        let bytecode = Bytes::from(make_sstore_revert_bytecode());
        let code = Code::from_bytecode(bytecode);

        // Compile via JIT
        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        backend
            .compile_and_cache(&code, fork, &code_cache)
            .expect("JIT compilation should succeed");
        let compiled = code_cache
            .get(&(code.hash, fork))
            .expect("compiled code should be in cache");

        // Pre-seed storage: slot 0 = 5
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let (contract_addr, sender_addr, accounts) = make_contract_accounts(code.clone(), storage);
        let mut db = make_test_db(accounts);
        let env = make_test_env(sender_addr);

        #[expect(clippy::as_conversions)]
        let mut call_frame = ethrex_levm::call_frame::CallFrame::new(
            sender_addr,
            contract_addr,
            contract_addr,
            code,
            U256::zero(),
            Bytes::new(),
            false,
            TEST_GAS_LIMIT,
            0,
            false,
            false,
            0,
            0,
            ethrex_levm::call_frame::Stack::default(),
            ethrex_levm::memory::Memory::default(),
        );

        let mut substate = ethrex_levm::vm::Substate::default();
        let mut storage_original_values = FxHashMap::default();

        let outcome = execute_jit(
            &compiled,
            &mut call_frame,
            &mut db,
            &mut substate,
            &env,
            &mut storage_original_values,
        )
        .expect("JIT execution should not error");

        // Outcome must be Revert
        assert!(
            matches!(outcome, ethrex_levm::jit::types::JitOutcome::Revert { .. }),
            "Expected Revert, got: {outcome:?}"
        );

        // Storage slot 0 must be restored to 5 (not 0x42)
        let slot_val = db
            .current_accounts_state
            .get(&contract_addr)
            .and_then(|a| a.storage.get(&H256::zero()).copied())
            .expect("slot 0 should exist");
        assert_eq!(
            slot_val,
            U256::from(5u64),
            "Storage slot 0 should be rolled back to 5, got {slot_val}"
        );
    }

    /// Verify multi-SSTORE→REVERT rollback restores original value.
    ///
    /// Pre-seeds slot 0 = 5, writes slot 0 = 10, 20, 30, then REVERTs.
    /// Journal must replay in reverse: restore 20→10→5. Final value = 5.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_multi_sstore_revert_rollback() {
        use ethrex_levm::{jit::cache::CodeCache, vm::JIT_STATE};

        use crate::backend::RevmcBackend;
        use crate::execution::execute_jit;

        JIT_STATE.reset_for_testing();

        let fork = ethrex_common::types::Fork::Cancun;

        let bytecode = Bytes::from(make_multi_sstore_revert_bytecode());
        let code = Code::from_bytecode(bytecode);

        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        backend
            .compile_and_cache(&code, fork, &code_cache)
            .expect("JIT compilation should succeed");
        let compiled = code_cache
            .get(&(code.hash, fork))
            .expect("compiled code should be in cache");

        // Pre-seed storage: slot 0 = 5
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let (contract_addr, sender_addr, accounts) = make_contract_accounts(code.clone(), storage);
        let mut db = make_test_db(accounts);
        let env = make_test_env(sender_addr);

        #[expect(clippy::as_conversions)]
        let mut call_frame = ethrex_levm::call_frame::CallFrame::new(
            sender_addr,
            contract_addr,
            contract_addr,
            code,
            U256::zero(),
            Bytes::new(),
            false,
            TEST_GAS_LIMIT,
            0,
            false,
            false,
            0,
            0,
            ethrex_levm::call_frame::Stack::default(),
            ethrex_levm::memory::Memory::default(),
        );

        let mut substate = ethrex_levm::vm::Substate::default();
        let mut storage_original_values = FxHashMap::default();

        let outcome = execute_jit(
            &compiled,
            &mut call_frame,
            &mut db,
            &mut substate,
            &env,
            &mut storage_original_values,
        )
        .expect("JIT execution should not error");

        assert!(
            matches!(outcome, ethrex_levm::jit::types::JitOutcome::Revert { .. }),
            "Expected Revert, got: {outcome:?}"
        );

        // Storage slot 0 must be restored to 5 (not 10, 20, or 30)
        let slot_val = db
            .current_accounts_state
            .get(&contract_addr)
            .and_then(|a| a.storage.get(&H256::zero()).copied())
            .expect("slot 0 should exist");
        assert_eq!(
            slot_val,
            U256::from(5u64),
            "Storage slot 0 should be rolled back to 5 after 3 SSTOREs + REVERT, got {slot_val}"
        );
    }

    /// Verify that JIT SSTORE→REVERT rollback works across different slots.
    ///
    /// Pre-seeds slot 0 = 5, slot 1 = 7. Runs bytecode that writes
    /// slot 0 = 10 and slot 1 = 20, then REVERTs. Both slots must be restored.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_two_slot_sstore_revert_rollback() {
        use ethrex_levm::{jit::cache::CodeCache, vm::JIT_STATE};

        use crate::backend::RevmcBackend;
        use crate::execution::execute_jit;

        JIT_STATE.reset_for_testing();

        let fork = ethrex_common::types::Fork::Cancun;

        let bytecode = Bytes::from(make_two_slot_sstore_revert_bytecode());
        let code = Code::from_bytecode(bytecode);

        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        backend
            .compile_and_cache(&code, fork, &code_cache)
            .expect("JIT compilation should succeed");
        let compiled = code_cache
            .get(&(code.hash, fork))
            .expect("compiled code should be in cache");

        // Pre-seed storage: slot 0 = 5, slot 1 = 7
        let slot_1 = H256::from_low_u64_be(1);
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));
        storage.insert(slot_1, U256::from(7u64));

        let contract_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);
        let accounts = vec![
            TestAccount {
                address: contract_addr,
                code: code.clone(),
                storage,
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ];
        let mut db = make_test_db(accounts);
        let env = make_test_env(sender_addr);

        #[expect(clippy::as_conversions)]
        let mut call_frame = ethrex_levm::call_frame::CallFrame::new(
            sender_addr,
            contract_addr,
            contract_addr,
            code,
            U256::zero(),
            Bytes::new(),
            false,
            TEST_GAS_LIMIT,
            0,
            false,
            false,
            0,
            0,
            ethrex_levm::call_frame::Stack::default(),
            ethrex_levm::memory::Memory::default(),
        );

        let mut substate = ethrex_levm::vm::Substate::default();
        let mut storage_original_values = FxHashMap::default();

        let outcome = execute_jit(
            &compiled,
            &mut call_frame,
            &mut db,
            &mut substate,
            &env,
            &mut storage_original_values,
        )
        .expect("JIT execution should not error");

        assert!(
            matches!(outcome, ethrex_levm::jit::types::JitOutcome::Revert { .. }),
            "Expected Revert, got: {outcome:?}"
        );

        // Slot 0 must be restored to 5 (not 10)
        let slot0_val = db
            .current_accounts_state
            .get(&contract_addr)
            .and_then(|a| a.storage.get(&H256::zero()).copied())
            .expect("slot 0 should exist");
        assert_eq!(
            slot0_val,
            U256::from(5u64),
            "Storage slot 0 should be rolled back to 5, got {slot0_val}"
        );

        // Slot 1 must be restored to 7 (not 20)
        let slot1_val = db
            .current_accounts_state
            .get(&contract_addr)
            .and_then(|a| a.storage.get(&slot_1).copied())
            .expect("slot 1 should exist");
        assert_eq!(
            slot1_val,
            U256::from(7u64),
            "Storage slot 1 should be rolled back to 7, got {slot1_val}"
        );
    }

    /// Verify that JIT gas_used matches interpreter gas_used for the counter contract.
    ///
    /// Uses apply_jit_outcome's formula (gas_limit - max(gas_remaining, 0))
    /// and compares against the interpreter's stateless_execute result.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_gas_matches_interpreter() {
        use ethrex_levm::{
            jit::cache::CodeCache,
            vm::{JIT_STATE, VM, VMType},
        };

        use crate::backend::RevmcBackend;
        use crate::execution::execute_jit;

        JIT_STATE.reset_for_testing();

        let fork = ethrex_common::types::Fork::Cancun;

        let bytecode = Bytes::from(make_counter_bytecode());
        let counter_code = Code::from_bytecode(bytecode);

        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        backend
            .compile_and_cache(&counter_code, fork, &code_cache)
            .expect("JIT compilation should succeed");
        let compiled = code_cache
            .get(&(counter_code.hash, fork))
            .expect("compiled code should be in cache");

        // Pre-seed storage: slot 0 = 5
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        // --- Interpreter path ---
        let (contract_addr, sender_addr, interp_accounts) =
            make_contract_accounts(counter_code.clone(), storage.clone());
        let mut interp_db = make_test_db(interp_accounts);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(
            env.clone(),
            &mut interp_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("VM::new should succeed");
        let interp_report = vm.stateless_execute().expect("interpreter should succeed");
        assert!(interp_report.is_success());

        // --- JIT direct execution path ---
        let (_, _, jit_accounts) = make_contract_accounts(counter_code.clone(), storage);
        let mut jit_db = make_test_db(jit_accounts);

        #[expect(clippy::as_conversions)]
        let mut call_frame = ethrex_levm::call_frame::CallFrame::new(
            sender_addr,
            contract_addr,
            contract_addr,
            counter_code,
            U256::zero(),
            Bytes::new(),
            false,
            TEST_GAS_LIMIT,
            0,
            false,
            false,
            0,
            0,
            ethrex_levm::call_frame::Stack::default(),
            ethrex_levm::memory::Memory::default(),
        );

        let mut substate = ethrex_levm::vm::Substate::default();
        let mut storage_original_values = FxHashMap::default();

        let outcome = execute_jit(
            &compiled,
            &mut call_frame,
            &mut jit_db,
            &mut substate,
            &env,
            &mut storage_original_values,
        )
        .expect("JIT execution should succeed");

        // Compute gas_used using apply_jit_outcome's formula:
        // gas_used = gas_limit - max(gas_remaining, 0)
        #[expect(clippy::as_conversions)]
        let jit_gas_remaining = call_frame.gas_remaining.max(0) as u64;
        let jit_execution_gas = TEST_GAS_LIMIT
            .checked_sub(jit_gas_remaining)
            .expect("gas_limit >= gas_remaining");

        match outcome {
            ethrex_levm::jit::types::JitOutcome::Success { gas_used, .. } => {
                // Sanity: formula from call_frame matches JitOutcome::gas_used
                assert_eq!(
                    jit_execution_gas, gas_used,
                    "apply_jit_outcome formula ({jit_execution_gas}) != \
                     JitOutcome::gas_used ({gas_used})"
                );

                // Cross-check: JIT execution gas + INTRINSIC_GAS == interpreter gas_used.
                // The interpreter's stateless_execute() includes INTRINSIC_GAS
                // for a basic EIP-1559 CALL. The JIT's gas_used is execution-only
                // (intrinsic gas was deducted before entering execute_jit). So:
                //   interp_report.gas_used == jit_execution_gas + INTRINSIC_GAS
                let interp_gas = interp_report.gas_used;
                assert_eq!(
                    interp_gas,
                    jit_execution_gas
                        .checked_add(INTRINSIC_GAS)
                        .expect("no overflow"),
                    "interpreter gas_used ({interp_gas}) != JIT execution gas \
                     ({jit_execution_gas}) + intrinsic ({INTRINSIC_GAS})"
                );
            }
            other => panic!("Expected JIT success, got: {other:?}"),
        }
    }
}
