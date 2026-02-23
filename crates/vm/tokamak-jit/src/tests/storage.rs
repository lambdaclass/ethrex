//! SLOAD/SSTORE E2E test for the JIT compiler.
//!
//! Tests a simple counter contract that reads storage slot 0, increments it,
//! writes it back, and returns the new value. Validates that JIT execution
//! produces identical output and gas usage to the interpreter.
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
        use std::sync::Arc;

        use ethrex_common::{
            Address, U256,
            constants::EMPTY_TRIE_HASH,
            types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
        };
        use ethrex_levm::{
            Environment,
            db::gen_db::GeneralizedDatabase,
            tracing::LevmCallTracer,
            vm::{VM, VMType},
        };
        use rustc_hash::FxHashMap;

        let contract_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);

        let bytecode = Bytes::from(make_counter_bytecode());
        let counter_code = Code::from_bytecode(bytecode);

        // Pre-seed storage: slot 0 = 5
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header)
                .expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            contract_addr,
            Account::new(U256::MAX, counter_code.clone(), 0, storage),
        );
        cache.insert(
            sender_addr,
            Account::new(
                U256::MAX,
                Code::from_bytecode(Bytes::new()),
                0,
                FxHashMap::default(),
            ),
        );
        let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache);

        let env = Environment {
            origin: sender_addr,
            #[expect(clippy::as_conversions)]
            gas_limit: (i64::MAX - 1) as u64,
            #[expect(clippy::as_conversions)]
            block_gas_limit: (i64::MAX - 1) as u64,
            ..Default::default()
        };
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(contract_addr),
            data: Bytes::new(),
            ..Default::default()
        });

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
    fn test_counter_jit_vs_interpreter() {
        use std::sync::Arc;

        use ethrex_common::{
            Address, U256,
            constants::EMPTY_TRIE_HASH,
            types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
        };
        use ethrex_levm::{
            Environment,
            db::gen_db::GeneralizedDatabase,
            jit::cache::CodeCache,
            tracing::LevmCallTracer,
            vm::{VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::execution::execute_jit;

        let contract_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);
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
        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header)
                .expect("StoreVmDatabase"),
        );
        let mut interp_cache = FxHashMap::default();
        interp_cache.insert(
            contract_addr,
            Account::new(U256::MAX, counter_code.clone(), 0, storage.clone()),
        );
        interp_cache.insert(
            sender_addr,
            Account::new(
                U256::MAX,
                Code::from_bytecode(Bytes::new()),
                0,
                FxHashMap::default(),
            ),
        );
        let mut interp_db =
            GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), interp_cache);

        let env = Environment {
            origin: sender_addr,
            #[expect(clippy::as_conversions)]
            gas_limit: (i64::MAX - 1) as u64,
            #[expect(clippy::as_conversions)]
            block_gas_limit: (i64::MAX - 1) as u64,
            ..Default::default()
        };
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(contract_addr),
            data: Bytes::new(),
            ..Default::default()
        });

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
        let store2 = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header2 = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db2: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store2, header2)
                .expect("StoreVmDatabase"),
        );
        let mut jit_account_cache = FxHashMap::default();
        jit_account_cache.insert(
            contract_addr,
            Account::new(U256::MAX, counter_code.clone(), 0, storage),
        );
        jit_account_cache.insert(
            sender_addr,
            Account::new(
                U256::MAX,
                Code::from_bytecode(Bytes::new()),
                0,
                FxHashMap::default(),
            ),
        );
        let mut jit_db =
            GeneralizedDatabase::new_with_account_state(Arc::new(vm_db2), jit_account_cache);

        #[expect(clippy::as_conversions)]
        let mut call_frame = ethrex_levm::call_frame::CallFrame::new(
            sender_addr,
            contract_addr,
            contract_addr,
            counter_code,
            U256::zero(),
            Bytes::new(),
            false,
            (i64::MAX - 1) as u64,
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
            ethrex_levm::jit::types::JitOutcome::Success {
                output, gas_used, ..
            } => {
                assert_eq!(
                    output, interp_report.output,
                    "JIT and interpreter output mismatch"
                );
                let jit_result = U256::from_big_endian(&output);
                assert_eq!(jit_result, U256::from(6u64), "JIT: 5 + 1 = 6");

                // Gas used should match between JIT and interpreter
                let interp_gas_used = interp_report.gas_used;
                assert_eq!(
                    gas_used, interp_gas_used,
                    "JIT gas_used ({gas_used}) != interpreter gas_used ({interp_gas_used})"
                );
            }
            other => {
                panic!("Expected JIT success, got: {other:?}");
            }
        }
    }
}
