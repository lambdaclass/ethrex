//! Fibonacci PoC test for the JIT compiler.
//!
//! This test uses hand-crafted EVM bytecode that computes Fibonacci numbers.
//! It verifies the JIT infrastructure (analysis, caching) and runs the
//! bytecode through the LEVM interpreter to validate correctness.
#![allow(clippy::vec_init_then_push)]
//!
//! When the `revmc-backend` feature is enabled, it additionally compiles
//! the bytecode via revmc/LLVM JIT and validates against the interpreter.

use bytes::Bytes;
use ethrex_common::H256;
use ethrex_levm::jit::{analyzer::analyze_bytecode, cache::CodeCache, counter::ExecutionCounter};

/// Build Fibonacci EVM bytecode that reads n from calldata[0..32] and
/// returns fib(n) as a 32-byte big-endian value in memory[0..32].
///
/// Uses only pure computation opcodes: PUSH, DUP, SWAP, ADD, SUB, LT,
/// ISZERO, JUMP, JUMPI, JUMPDEST, CALLDATALOAD, MSTORE, RETURN, POP, STOP.
///
/// fib(0) = 0, fib(1) = 1, fib(n) = fib(n-1) + fib(n-2) for n >= 2.
pub fn make_fibonacci_bytecode() -> Vec<u8> {
    let mut code = Vec::new();

    // === SECTION 1: Load n and branch (offsets 0..10) ===
    code.push(0x60);
    code.push(0x00); //  0: PUSH1 0
    code.push(0x35); //  2: CALLDATALOAD → [n]
    code.push(0x80); //  3: DUP1 → [n, n]
    code.push(0x60);
    code.push(0x02); //  4: PUSH1 2
    // GT: pops a=2, b=n, pushes (2 > n) i.e. (n < 2)
    code.push(0x11); //  6: GT → [n < 2, n]
    code.push(0x15); //  7: ISZERO → [n >= 2, n]
    code.push(0x60);
    code.push(0x13); //  8: PUSH1 19
    code.push(0x57); // 10: JUMPI → if n>=2, goto offset 19

    // === SECTION 2: Base case — return n (offsets 11..18) ===
    // Stack: [n]
    code.push(0x60);
    code.push(0x00); // 11: PUSH1 0
    code.push(0x52); // 13: MSTORE → mem[0..32] = n
    code.push(0x60);
    code.push(0x20); // 14: PUSH1 32
    code.push(0x60);
    code.push(0x00); // 16: PUSH1 0
    code.push(0xf3); // 18: RETURN

    // === SECTION 3: Loop setup (offset 19 = 0x13) ===
    code.push(0x5b); // 19: JUMPDEST
    // Stack: [n], n >= 2
    // Initialize: counter=n, curr=1, prev=0
    code.push(0x60);
    code.push(0x01); // 20: PUSH1 1 → [1, n]
    code.push(0x60);
    code.push(0x00); // 22: PUSH1 0 → [0, 1, n]
    code.push(0x91); // 24: SWAP2 → [n, 1, 0]
    // Stack: [counter=n, curr=1, prev=0]

    // === SECTION 4: Loop body (offset 25 = 0x19) ===
    code.push(0x5b); // 25: JUMPDEST
    // Stack: [counter, curr, prev]
    // new_curr = curr + prev
    code.push(0x81); // 26: DUP2 → [curr, counter, curr, prev]
    code.push(0x83); // 27: DUP4 → [prev, curr, counter, curr, prev]
    code.push(0x01); // 28: ADD → [curr+prev, counter, curr, prev]
    // Stack: [new_curr, counter, old_curr, old_prev]
    // Drop old_prev: SWAP3 + POP
    code.push(0x92); // 29: SWAP3 → [old_prev, counter, old_curr, new_curr]
    code.push(0x50); // 30: POP → [counter, old_curr, new_curr]
    // Stack: [counter, new_prev=old_curr, new_curr]
    // Decrement counter
    code.push(0x60);
    code.push(0x01); // 31: PUSH1 1 → [1, counter, new_prev, new_curr]
    code.push(0x90); // 33: SWAP1 → [counter, 1, new_prev, new_curr]
    code.push(0x03); // 34: SUB → [counter-1, new_prev, new_curr]
    // Rearrange to [counter-1, new_curr, new_prev]
    code.push(0x91); // 35: SWAP2 → [new_curr, new_prev, counter-1]
    code.push(0x90); // 36: SWAP1 → [new_prev, new_curr, counter-1]
    code.push(0x91); // 37: SWAP2 → [counter-1, new_curr, new_prev]
    // Stack: [counter-1, new_curr, new_prev] ✓
    // Check if counter-1 > 1: LT pops a=1, b=c-1, pushes (1 < c-1) ≡ (c-1 > 1)
    code.push(0x80); // 38: DUP1 → [c-1, c-1, new_curr, new_prev]
    code.push(0x60);
    code.push(0x01); // 39: PUSH1 1 → [1, c-1, c-1, ...]
    code.push(0x10); // 41: LT → [1 < (c-1), c-1, new_curr, new_prev]
    code.push(0x60);
    code.push(0x19); // 42: PUSH1 25 → [25, cond, c-1, new_curr, new_prev]
    code.push(0x57); // 44: JUMPI → if (c-1)>1, goto loop body

    // === SECTION 5: Return curr (offsets 45..55) ===
    // Stack: [counter-1, new_curr, new_prev]
    code.push(0x50); // 45: POP → [new_curr, new_prev]
    code.push(0x90); // 46: SWAP1 → [new_prev, new_curr]
    code.push(0x50); // 47: POP → [new_curr]
    code.push(0x60);
    code.push(0x00); // 48: PUSH1 0
    code.push(0x52); // 50: MSTORE
    code.push(0x60);
    code.push(0x20); // 51: PUSH1 32
    code.push(0x60);
    code.push(0x00); // 53: PUSH1 0
    code.push(0xf3); // 55: RETURN

    code
}

/// Expected Fibonacci values for testing.
const FIBONACCI_VALUES: [(u64, u64); 11] = [
    (0, 0),
    (1, 1),
    (2, 1),
    (3, 2),
    (4, 3),
    (5, 5),
    (6, 8),
    (7, 13),
    (8, 21),
    (10, 55),
    (20, 6765),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fibonacci_bytecode_is_valid() {
        let code = make_fibonacci_bytecode();
        assert!(!code.is_empty());
        assert!(code.contains(&0x5b), "should contain JUMPDEST");
        assert_eq!(code.last(), Some(&0xf3), "should end with RETURN");
    }

    #[test]
    fn test_fibonacci_bytecode_analysis() {
        let bytecode = Bytes::from(make_fibonacci_bytecode());
        let analyzed = analyze_bytecode(bytecode, H256::zero(), vec![19, 25]);

        assert!(
            analyzed.basic_blocks.len() >= 3,
            "should have >= 3 basic blocks, got {}",
            analyzed.basic_blocks.len()
        );
        assert!(analyzed.opcode_count > 10, "should have > 10 opcodes");
    }

    #[test]
    fn test_cache_workflow() {
        use ethrex_common::types::Fork;

        let cache = CodeCache::new();
        let counter = ExecutionCounter::new();
        let hash = H256::from_low_u64_be(42);
        let fork = Fork::Cancun;

        for _ in 0..10 {
            counter.increment(&hash);
        }
        assert_eq!(counter.get(&hash), 10);

        let key = (hash, fork);
        assert!(cache.get(&key).is_none());
        assert!(cache.is_empty());

        #[expect(unsafe_code)]
        let compiled =
            unsafe { ethrex_levm::jit::cache::CompiledCode::new(std::ptr::null(), 100, 5, None, false) };
        cache.insert(key, compiled);
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.len(), 1);
    }

    /// Compile Fibonacci bytecode via revmc/LLVM, register the JIT backend,
    /// then execute through the full VM dispatch path (vm.rs → JIT → host).
    ///
    /// This is the Phase 3 E2E test: bytecode is pre-compiled, inserted into
    /// the cache, and the VM's JIT dispatch picks it up instead of interpreting.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_fibonacci_jit_execution() {
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
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;

        let contract_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);

        let bytecode = Bytes::from(make_fibonacci_bytecode());
        let fib_code = Code::from_bytecode(bytecode);

        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        // 1. Compile Fibonacci bytecode via RevmcBackend
        let backend = RevmcBackend::default();
        let fork = ethrex_common::types::Fork::Cancun;
        backend
            .compile_and_cache(&fib_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation should succeed");
        assert!(
            JIT_STATE.cache.get(&(fib_code.hash, fork)).is_some(),
            "compiled code should be in cache"
        );

        // 2. Register the backend for JIT execution
        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        // 3. Run through VM — the JIT dispatch should pick up the cached code
        for (n, expected_fib) in FIBONACCI_VALUES {
            let mut calldata = vec![0u8; 32];
            calldata[24..32].copy_from_slice(&n.to_be_bytes());
            let calldata = Bytes::from(calldata);

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
                Account::new(U256::MAX, fib_code.clone(), 0, FxHashMap::default()),
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
                data: calldata,
                ..Default::default()
            });

            let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
                .unwrap_or_else(|e| panic!("VM::new failed for fib({n}): {e:?}"));

            let report = vm
                .stateless_execute()
                .unwrap_or_else(|e| panic!("JIT fib({n}) execution failed: {e:?}"));

            assert!(
                report.is_success(),
                "JIT fib({n}) should succeed, got: {:?}",
                report.result
            );

            assert_eq!(
                report.output.len(),
                32,
                "JIT fib({n}) should return 32 bytes, got {}",
                report.output.len()
            );
            let result_val = U256::from_big_endian(&report.output);
            assert_eq!(
                result_val,
                U256::from(expected_fib),
                "JIT fib({n}) = {expected_fib}, got {result_val}"
            );
        }
    }

    /// Validate JIT execution produces identical results to the interpreter.
    ///
    /// Runs Fibonacci for each test value through both paths and compares
    /// output bytes and success status.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_fibonacci_jit_vs_interpreter_validation() {
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
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::execution::execute_jit;

        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        let contract_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);

        let bytecode = Bytes::from(make_fibonacci_bytecode());
        let fib_code = Code::from_bytecode(bytecode);

        // Compile the bytecode
        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        let fork = ethrex_common::types::Fork::Cancun;
        backend
            .compile_and_cache(&fib_code, fork, &code_cache)
            .expect("compilation should succeed");
        let compiled = code_cache
            .get(&(fib_code.hash, fork))
            .expect("compiled code should be in cache");

        for (n, expected_fib) in FIBONACCI_VALUES {
            let mut calldata = vec![0u8; 32];
            calldata[24..32].copy_from_slice(&n.to_be_bytes());
            let calldata = Bytes::from(calldata);

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
                Account::new(U256::MAX, fib_code.clone(), 0, FxHashMap::default()),
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
                data: calldata.clone(),
                ..Default::default()
            });

            let mut vm = VM::new(
                env.clone(),
                &mut interp_db,
                &tx,
                LevmCallTracer::disabled(),
                VMType::L1,
            )
            .unwrap_or_else(|e| panic!("Interpreter VM::new failed for fib({n}): {e:?}"));

            let interp_report = vm
                .stateless_execute()
                .unwrap_or_else(|e| panic!("Interpreter fib({n}) failed: {e:?}"));

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
                Account::new(U256::MAX, fib_code.clone(), 0, FxHashMap::default()),
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

            // Build a minimal CallFrame matching what the VM would create
            #[expect(clippy::as_conversions)]
            let mut call_frame = ethrex_levm::call_frame::CallFrame::new(
                sender_addr,   // msg_sender
                contract_addr, // to
                contract_addr, // code_address
                fib_code.clone(),
                U256::zero(), // msg_value
                calldata,
                false,                 // is_static
                (i64::MAX - 1) as u64, // gas_limit
                0,                     // depth
                false,                 // should_transfer_value
                false,                 // is_create
                0,                     // ret_offset
                0,                     // ret_size
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
            .unwrap_or_else(|e| panic!("JIT fib({n}) execution failed: {e:?}"));

            // Compare results
            match jit_outcome {
                ethrex_levm::jit::types::JitOutcome::Success { output, gas_used } => {
                    assert!(
                        interp_report.is_success(),
                        "fib({n}): JIT succeeded but interpreter didn't: {:?}",
                        interp_report.result
                    );
                    assert_eq!(
                        output, interp_report.output,
                        "fib({n}): JIT and interpreter output mismatch"
                    );
                    assert_eq!(
                        gas_used, interp_report.gas_used,
                        "fib({n}): JIT and interpreter gas_used mismatch"
                    );
                    let result_val = U256::from_big_endian(&output);
                    assert_eq!(
                        result_val,
                        U256::from(expected_fib),
                        "fib({n}) validation: expected {expected_fib}, got {result_val}"
                    );
                }
                other => {
                    panic!("fib({n}): expected JIT success, got: {other:?}");
                }
            }
        }
    }

    /// Run Fibonacci bytecode through the LEVM interpreter and verify results.
    ///
    /// This validates the hand-crafted bytecode is correct and produces
    /// the expected Fibonacci sequence values.
    #[test]
    fn test_fibonacci_interpreter_execution() {
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

        let bytecode = Bytes::from(make_fibonacci_bytecode());
        let fib_code = Code::from_bytecode(bytecode);

        for (n, expected_fib) in FIBONACCI_VALUES {
            // Build calldata: n as 32-byte big-endian (no selector, direct calldataload)
            let mut calldata = vec![0u8; 32];
            calldata[24..32].copy_from_slice(&n.to_be_bytes());
            let calldata = Bytes::from(calldata);

            // Create in-memory database with contract and sender accounts
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
                Account::new(U256::MAX, fib_code.clone(), 0, FxHashMap::default()),
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

            // Create VM
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
                data: calldata,
                ..Default::default()
            });

            let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
                .unwrap_or_else(|e| panic!("VM::new failed for fib({n}): {e:?}"));

            let report = vm
                .stateless_execute()
                .unwrap_or_else(|e| panic!("fib({n}) execution failed: {e:?}"));

            assert!(
                report.is_success(),
                "fib({n}) should succeed, got: {:?}",
                report.result
            );

            // Parse output as U256 (big-endian)
            assert_eq!(
                report.output.len(),
                32,
                "fib({n}) should return 32 bytes, got {}",
                report.output.len()
            );
            let result_val = U256::from_big_endian(&report.output);
            assert_eq!(
                result_val,
                U256::from(expected_fib),
                "fib({n}) = {expected_fib}, got {result_val}"
            );
        }
    }
}
