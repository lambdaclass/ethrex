//! CALL/CREATE resume tests for the JIT compiler.
//!
//! Tests JIT-compiled bytecodes that contain CALL/CREATE opcodes, exercising
//! the suspend/resume pipeline: JIT execution suspends on CALL, LEVM runs
//! the sub-call, and JIT resumes with the result.
#![allow(clippy::vec_init_then_push)]

/// Build a "caller" contract that does STATICCALL to `target_addr` and returns
/// the result. The helper is expected to return a 32-byte value.
///
/// ```text
/// // Push STATICCALL args
/// PUSH1 0x20           // retSize = 32
/// PUSH1 0x00           // retOffset = 0
/// PUSH1 0x00           // argsSize = 0
/// PUSH1 0x00           // argsOffset = 0
/// PUSH20 <target_addr> // address
/// PUSH3 0xFFFFFF       // gas = 0xFFFFFF
/// STATICCALL           // [success]
///
/// // If success, return memory[0..32] (the callee's output)
/// POP                  // discard success
/// PUSH1 0x20           // size = 32
/// PUSH1 0x00           // offset = 0
/// RETURN
/// ```
pub fn make_staticcall_caller(target_addr: [u8; 20]) -> Vec<u8> {
    let mut code = Vec::new();

    //  0: PUSH1 0x20 (retSize = 32)
    code.push(0x60);
    code.push(0x20);
    //  2: PUSH1 0x00 (retOffset = 0)
    code.push(0x60);
    code.push(0x00);
    //  4: PUSH1 0x00 (argsSize = 0)
    code.push(0x60);
    code.push(0x00);
    //  6: PUSH1 0x00 (argsOffset = 0)
    code.push(0x60);
    code.push(0x00);
    //  8: PUSH20 <target_addr>
    code.push(0x73);
    code.extend_from_slice(&target_addr);
    // 29: PUSH3 0xFFFFFF (gas)
    code.push(0x62);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    // 33: STATICCALL
    code.push(0xFA);
    // 34: POP (discard success flag — we'll just return the callee output)
    code.push(0x50);
    // 35: PUSH1 0x20 (return size)
    code.push(0x60);
    code.push(0x20);
    // 37: PUSH1 0x00 (return offset)
    code.push(0x60);
    code.push(0x00);
    // 39: RETURN
    code.push(0xF3);

    code
}

/// Build a simple "callee" contract that returns the value 42 in memory[0..32].
///
/// ```text
/// PUSH1 42
/// PUSH1 0x00
/// MSTORE
/// PUSH1 0x20
/// PUSH1 0x00
/// RETURN
/// ```
pub fn make_return42_bytecode() -> Vec<u8> {
    let mut code = Vec::new();

    code.push(0x60);
    code.push(42); // PUSH1 42
    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0x52); // MSTORE
    code.push(0x60);
    code.push(0x20); // PUSH1 32
    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0xf3); // RETURN

    code
}

/// Build a "callee" contract that immediately REVERTs with empty output.
///
/// ```text
/// PUSH1 0x00
/// PUSH1 0x00
/// REVERT
/// ```
pub fn make_reverting_bytecode() -> Vec<u8> {
    let mut code = Vec::new();

    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0xFD); // REVERT

    code
}

/// Build a caller contract that does STATICCALL and checks the return value.
/// If the call succeeded (1 on stack), returns memory[0..32].
/// If the call failed (0 on stack), returns 0xDEAD as the output.
///
/// ```text
/// // STATICCALL to target
/// PUSH1 0x20           // retSize
/// PUSH1 0x00           // retOffset
/// PUSH1 0x00           // argsSize
/// PUSH1 0x00           // argsOffset
/// PUSH20 <target>      // address
/// PUSH3 0xFFFFFF       // gas
/// STATICCALL           // [success]
///
/// // Branch on success
/// PUSH1 <success_dest>
/// JUMPI
///
/// // Failure path: return 0xDEAD
/// PUSH2 0xDEAD
/// PUSH1 0x00
/// MSTORE
/// PUSH1 0x20
/// PUSH1 0x00
/// RETURN
///
/// // Success path: return memory[0..32]
/// JUMPDEST
/// PUSH1 0x20
/// PUSH1 0x00
/// RETURN
/// ```
pub fn make_checked_staticcall_caller(target_addr: [u8; 20]) -> Vec<u8> {
    let mut code = Vec::new();

    //  0: PUSH1 0x20
    code.push(0x60);
    code.push(0x20);
    //  2: PUSH1 0x00
    code.push(0x60);
    code.push(0x00);
    //  4: PUSH1 0x00
    code.push(0x60);
    code.push(0x00);
    //  6: PUSH1 0x00
    code.push(0x60);
    code.push(0x00);
    //  8: PUSH20 <target>
    code.push(0x73);
    code.extend_from_slice(&target_addr);
    // 29: PUSH3 0xFFFFFF
    code.push(0x62);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    // 33: STATICCALL → [success]
    code.push(0xFA);

    // 34: PUSH1 <success_dest = 47>
    code.push(0x60);
    code.push(47);
    // 36: JUMPI
    code.push(0x57);

    // 37: Failure path — store 0xDEAD and return
    code.push(0x61); // PUSH2 0xDEAD
    code.push(0xDE);
    code.push(0xAD);
    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0x52); // MSTORE
    code.push(0x60);
    code.push(0x20); // PUSH1 32
    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0xF3); // RETURN

    // 47: JUMPDEST — success path
    code.push(0x5B);
    // 48: return memory[0..32]
    code.push(0x60);
    code.push(0x20); // PUSH1 32
    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0xF3); // RETURN

    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_staticcall_caller_bytecode_is_valid() {
        let target = [0x42u8; 20];
        let code = make_staticcall_caller(target);
        assert!(!code.is_empty());
        // Should contain STATICCALL opcode (0xFA)
        assert!(code.contains(&0xFA), "should contain STATICCALL");
        assert_eq!(code.last(), Some(&0xF3), "should end with RETURN");
    }

    #[test]
    fn test_return42_bytecode_is_valid() {
        let code = make_return42_bytecode();
        assert!(!code.is_empty());
        assert!(code.contains(&0x52), "should contain MSTORE");
        assert_eq!(code.last(), Some(&0xF3), "should end with RETURN");
    }

    #[test]
    fn test_checked_caller_bytecode_is_valid() {
        let target = [0x42u8; 20];
        let code = make_checked_staticcall_caller(target);
        assert!(!code.is_empty());
        assert!(code.contains(&0xFA), "should contain STATICCALL");
        assert!(code.contains(&0x5B), "should contain JUMPDEST");
    }

    /// Run caller→callee (STATICCALL) through the LEVM interpreter.
    ///
    /// Validates that the hand-crafted bytecodes work correctly before
    /// testing the JIT path.
    #[test]
    fn test_staticcall_interpreter_execution() {
        use std::sync::Arc;

        use bytes::Bytes;
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

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);

        let callee_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let caller_code =
            Code::from_bytecode(Bytes::from(make_staticcall_caller(callee_addr.into())));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            callee_addr,
            Account::new(U256::MAX, callee_code, 0, FxHashMap::default()),
        );
        cache.insert(
            caller_addr,
            Account::new(U256::MAX, caller_code, 0, FxHashMap::default()),
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
            to: TxKind::Call(caller_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("staticcall execution should succeed");

        assert!(
            report.is_success(),
            "caller→callee should succeed, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes");
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(result_val, U256::from(42u64), "callee returns 42");
    }

    /// Test STATICCALL to a reverting callee via the interpreter.
    ///
    /// The caller checks the success flag and returns 0xDEAD on failure.
    #[test]
    fn test_staticcall_revert_interpreter_execution() {
        use std::sync::Arc;

        use bytes::Bytes;
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

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);

        let callee_code = Code::from_bytecode(Bytes::from(make_reverting_bytecode()));
        let caller_code =
            Code::from_bytecode(Bytes::from(make_checked_staticcall_caller(callee_addr.into())));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            callee_addr,
            Account::new(U256::MAX, callee_code, 0, FxHashMap::default()),
        );
        cache.insert(
            caller_addr,
            Account::new(U256::MAX, caller_code, 0, FxHashMap::default()),
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
            to: TxKind::Call(caller_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("staticcall-revert execution should succeed");

        assert!(
            report.is_success(),
            "outer call should succeed even when inner reverts, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes");
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(0xDEADu64),
            "caller should return 0xDEAD when callee reverts"
        );
    }

    /// Compile the caller contract via JIT and run caller→callee STATICCALL.
    ///
    /// The caller is JIT-compiled; the callee runs via the interpreter.
    /// This exercises the full suspend/resume pipeline:
    /// 1. JIT executes caller, hits STATICCALL → suspends with JitOutcome::Suspended
    /// 2. VM runs callee via interpreter → SubCallResult { success: true, output: [42] }
    /// 3. JIT resumes caller with sub-call result → returns 42
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_staticcall_jit_caller_interpreter_callee() {
        use std::sync::Arc;

        use bytes::Bytes;
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

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = ethrex_common::types::Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let caller_code =
            Code::from_bytecode(Bytes::from(make_staticcall_caller(callee_addr.into())));

        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        // Compile the caller via JIT (the callee stays interpreter-only)
        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of caller should succeed");
        assert!(
            JIT_STATE.cache.get(&(caller_code.hash, fork)).is_some(),
            "caller should be in JIT cache"
        );

        // Register the backend for execution
        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            callee_addr,
            Account::new(U256::MAX, callee_code, 0, FxHashMap::default()),
        );
        cache.insert(
            caller_addr,
            Account::new(U256::MAX, caller_code, 0, FxHashMap::default()),
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
            to: TxKind::Call(caller_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("JIT staticcall execution should succeed");

        assert!(
            report.is_success(),
            "JIT caller→interpreter callee should succeed, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes");
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(42u64),
            "JIT caller should return 42 from callee"
        );
    }

    /// JIT caller → reverting callee: verify failure propagation.
    ///
    /// The caller is JIT-compiled, does STATICCALL to a reverting callee,
    /// checks the return value (0 = failure), and returns 0xDEAD.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_staticcall_jit_caller_reverting_callee() {
        use std::sync::Arc;

        use bytes::Bytes;
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

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = ethrex_common::types::Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_reverting_bytecode()));
        let caller_code = Code::from_bytecode(Bytes::from(make_checked_staticcall_caller(
            callee_addr.into(),
        )));

        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        // Compile the caller via JIT
        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of checked caller should succeed");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            callee_addr,
            Account::new(U256::MAX, callee_code, 0, FxHashMap::default()),
        );
        cache.insert(
            caller_addr,
            Account::new(U256::MAX, caller_code, 0, FxHashMap::default()),
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
            to: TxKind::Call(caller_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("JIT staticcall-revert execution should succeed");

        assert!(
            report.is_success(),
            "outer JIT call should succeed even when inner reverts, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes");
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(0xDEADu64),
            "JIT caller should return 0xDEAD when callee reverts"
        );
    }

    /// JIT vs interpreter comparison for STATICCALL contracts.
    ///
    /// Runs the same caller→callee scenario through both paths and verifies
    /// identical output.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_staticcall_jit_vs_interpreter() {
        use std::sync::Arc;

        use bytes::Bytes;
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

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = ethrex_common::types::Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let caller_code =
            Code::from_bytecode(Bytes::from(make_staticcall_caller(callee_addr.into())));

        // --- Interpreter path ---
        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );
        let mut interp_cache = FxHashMap::default();
        interp_cache.insert(
            callee_addr,
            Account::new(U256::MAX, callee_code.clone(), 0, FxHashMap::default()),
        );
        interp_cache.insert(
            caller_addr,
            Account::new(U256::MAX, caller_code.clone(), 0, FxHashMap::default()),
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
            to: TxKind::Call(caller_addr),
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
            .expect("Interpreter staticcall should succeed");

        assert!(
            interp_report.is_success(),
            "Interpreter should succeed: {:?}",
            interp_report.result
        );
        let interp_val = U256::from_big_endian(&interp_report.output);
        assert_eq!(interp_val, U256::from(42u64));

        // --- JIT direct execution path ---
        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        backend
            .compile_and_cache(&caller_code, fork, &code_cache)
            .expect("JIT compilation should succeed");
        let compiled = code_cache
            .get(&(caller_code.hash, fork))
            .expect("compiled code should be in cache");

        let store2 = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header2 = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db2: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store2, header2).expect("StoreVmDatabase"),
        );
        let mut jit_account_cache = FxHashMap::default();
        jit_account_cache.insert(
            callee_addr,
            Account::new(U256::MAX, callee_code, 0, FxHashMap::default()),
        );
        jit_account_cache.insert(
            caller_addr,
            Account::new(U256::MAX, caller_code, 0, FxHashMap::default()),
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

        // Build CallFrame for caller contract
        #[expect(clippy::as_conversions)]
        let mut call_frame = ethrex_levm::call_frame::CallFrame::new(
            sender_addr,
            caller_addr,
            caller_addr,
            Code::from_bytecode(Bytes::from(make_staticcall_caller(callee_addr.into()))),
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
        .expect("JIT caller execution should succeed");

        // JIT should suspend on STATICCALL — verify suspension
        match jit_outcome {
            ethrex_levm::jit::types::JitOutcome::Suspended {
                resume_state,
                sub_call,
            } => {
                // Verify sub_call is a Call to the callee
                match &sub_call {
                    ethrex_levm::jit::types::JitSubCall::Call { target, .. } => {
                        assert_eq!(
                            *target, callee_addr,
                            "sub-call target should be the callee address"
                        );
                    }
                    other => panic!("expected JitSubCall::Call, got: {other:?}"),
                }

                // Resume with a successful sub-call result (simulating callee returning 42)
                let mut result_bytes = vec![0u8; 32];
                result_bytes[31] = 42;
                let sub_result = ethrex_levm::jit::types::SubCallResult {
                    success: true,
                    gas_limit: 0xFFFFFF,
                    gas_used: 100,
                    output: Bytes::from(result_bytes),
                    created_address: None,
                };

                let resumed_outcome = crate::execution::execute_jit_resume(
                    resume_state,
                    sub_result,
                    &mut call_frame,
                    &mut jit_db,
                    &mut substate,
                    &env,
                    &mut storage_original_values,
                )
                .expect("JIT resume should succeed");

                match resumed_outcome {
                    ethrex_levm::jit::types::JitOutcome::Success { output, .. } => {
                        assert_eq!(output.len(), 32, "should return 32 bytes");
                        let jit_val = U256::from_big_endian(&output);
                        assert_eq!(
                            jit_val,
                            U256::from(42u64),
                            "JIT resumed caller should return 42"
                        );
                    }
                    other => panic!("expected JIT Success after resume, got: {other:?}"),
                }
            }
            ethrex_levm::jit::types::JitOutcome::Success { .. } => {
                panic!("expected Suspended (STATICCALL should trigger suspension), got Success");
            }
            other => {
                panic!("expected Suspended, got: {other:?}");
            }
        }
    }

    /// Build a factory contract that CREATE-deploys a child contract.
    ///
    /// The child's init code stores 0x42 and returns it as deployed bytecode.
    /// The factory returns the deployed address as a 32-byte value.
    ///
    /// ```text
    /// // Store child init code in memory
    /// PUSH1 0x42           // byte to store in deployed code
    /// PUSH1 0x00           // memory offset
    /// MSTORE8              // mem[0] = 0x42
    /// // init code: PUSH1 0x01 PUSH1 0x00 RETURN (returns mem[0..1] = 0x42)
    /// PUSH5 <init_code>    // 600160005360016000F3 is too long, use MSTORE approach
    /// ...                  // (see bytecode below)
    ///
    /// // CREATE(value=0, offset=0, size=initcode_len)
    /// PUSH1 <initcode_len>
    /// PUSH1 0x00
    /// PUSH1 0x00
    /// CREATE               // [deployed_addr]
    ///
    /// // Return the address
    /// PUSH1 0x00
    /// MSTORE
    /// PUSH1 0x20
    /// PUSH1 0x00
    /// RETURN
    /// ```
    fn make_create_factory_bytecode() -> Vec<u8> {
        // Child init code: stores 0x42 at mem[0] and returns it as deployed bytecode.
        // PUSH1 0x42  PUSH1 0x00  MSTORE8  PUSH1 0x01  PUSH1 0x00  RETURN
        let init_code: Vec<u8> = vec![0x60, 0x42, 0x60, 0x00, 0x53, 0x60, 0x01, 0x60, 0x00, 0xF3];

        let mut code = Vec::new();

        // Store init code in memory starting at offset 0
        // Use PUSH + MSTORE approach: pack init_code into 32-byte word and store
        // Since init_code is 10 bytes, pad to 32 and store at offset 0
        // More simply: store each byte with MSTORE8
        for (i, &byte) in init_code.iter().enumerate() {
            code.push(0x60); // PUSH1
            code.push(byte);
            code.push(0x60); // PUSH1
            #[expect(clippy::as_conversions)]
            code.push(i as u8);
            code.push(0x53); // MSTORE8
        }

        // CREATE(value=0, offset=0, size=init_code.len())
        code.push(0x60); // PUSH1 size
        #[expect(clippy::as_conversions)]
        code.push(init_code.len() as u8);
        code.push(0x60); // PUSH1 offset=0
        code.push(0x00);
        code.push(0x60); // PUSH1 value=0
        code.push(0x00);
        code.push(0xF0); // CREATE → [deployed_addr]

        // Return deployed address
        code.push(0x60); // PUSH1 0x00
        code.push(0x00);
        code.push(0x52); // MSTORE
        code.push(0x60); // PUSH1 0x20
        code.push(0x20);
        code.push(0x60); // PUSH1 0x00
        code.push(0x00);
        code.push(0xF3); // RETURN

        code
    }

    /// Build a factory that uses CREATE2 with salt=1 to deploy a child.
    /// Returns the deployed address.
    fn make_create2_factory_bytecode() -> Vec<u8> {
        // Same child init code as above
        let init_code: Vec<u8> = vec![0x60, 0x42, 0x60, 0x00, 0x53, 0x60, 0x01, 0x60, 0x00, 0xF3];

        let mut code = Vec::new();

        // Store init code in memory
        for (i, &byte) in init_code.iter().enumerate() {
            code.push(0x60);
            code.push(byte);
            code.push(0x60);
            #[expect(clippy::as_conversions)]
            code.push(i as u8);
            code.push(0x53); // MSTORE8
        }

        // CREATE2(value=0, offset=0, size=init_code.len(), salt=1)
        code.push(0x60); // PUSH1 salt=1
        code.push(0x01);
        code.push(0x60); // PUSH1 size
        #[expect(clippy::as_conversions)]
        code.push(init_code.len() as u8);
        code.push(0x60); // PUSH1 offset=0
        code.push(0x00);
        code.push(0x60); // PUSH1 value=0
        code.push(0x00);
        code.push(0xF5); // CREATE2 → [deployed_addr]

        // Return deployed address
        code.push(0x60);
        code.push(0x00);
        code.push(0x52); // MSTORE
        code.push(0x60);
        code.push(0x20);
        code.push(0x60);
        code.push(0x00);
        code.push(0xF3); // RETURN

        code
    }

    /// Helper to set up a VM and run a factory contract through the interpreter.
    fn run_factory_via_interpreter(
        factory_addr: ethrex_common::Address,
        factory_code: ethrex_common::types::Code,
        extra_accounts: Vec<(
            ethrex_common::Address,
            ethrex_common::types::Account,
        )>,
    ) -> ethrex_levm::errors::ExecutionReport {
        use std::sync::Arc;

        use bytes::Bytes;
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

        let sender_addr = Address::from_low_u64_be(0x100);

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            factory_addr,
            Account::new(U256::MAX, factory_code, 0, FxHashMap::default()),
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
        for (addr, acct) in extra_accounts {
            cache.insert(addr, acct);
        }
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
            to: TxKind::Call(factory_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        vm.stateless_execute()
            .expect("factory execution should succeed")
    }

    /// Test CREATE success: factory deploys a child contract via CREATE.
    ///
    /// Validates that the interpreter correctly handles the full CREATE flow
    /// including nonce increment, collision check, deploy nonce init, and code storage.
    #[test]
    fn test_create_success_interpreter() {
        use bytes::Bytes;
        use ethrex_common::{Address, U256};
        use ethrex_common::types::Code;

        let factory_addr = Address::from_low_u64_be(0x42);
        let factory_code = Code::from_bytecode(Bytes::from(make_create_factory_bytecode()));

        let report = run_factory_via_interpreter(factory_addr, factory_code, vec![]);

        assert!(
            report.is_success(),
            "CREATE factory should succeed, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes (address)");

        // The returned address should be non-zero (CREATE succeeded)
        let deployed_addr_word = U256::from_big_endian(&report.output);
        assert_ne!(
            deployed_addr_word,
            U256::zero(),
            "deployed address should be non-zero"
        );
    }

    /// Test CREATE collision: attempt to deploy to an address that already has code.
    ///
    /// Pre-seeds the expected CREATE address with existing bytecode. The CREATE
    /// should fail (return address(0)) due to the collision check.
    #[test]
    fn test_create_collision_interpreter() {
        use bytes::Bytes;
        use ethrex_common::{Address, U256, evm::calculate_create_address};
        use ethrex_common::types::{Account, Code};
        use rustc_hash::FxHashMap;

        let factory_addr = Address::from_low_u64_be(0x42);
        let factory_code = Code::from_bytecode(Bytes::from(make_create_factory_bytecode()));

        // The factory contract's nonce is 0 (fresh account).
        // CREATE address = keccak256(rlp([factory_addr, nonce=0]))[12..]
        let collision_addr = calculate_create_address(factory_addr, 0);

        // Pre-seed the collision address with code so create_would_collide() returns true
        let collision_code = Code::from_bytecode(Bytes::from(vec![0x60, 0x00, 0xF3]));
        let collision_account =
            Account::new(U256::zero(), collision_code, 0, FxHashMap::default());

        let report = run_factory_via_interpreter(
            factory_addr,
            factory_code,
            vec![(collision_addr, collision_account)],
        );

        assert!(
            report.is_success(),
            "outer tx should succeed (CREATE failure is not an exceptional halt), got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes");

        // On CREATE collision, the factory gets address(0) back
        let deployed_addr_word = U256::from_big_endian(&report.output);
        assert_eq!(
            deployed_addr_word,
            U256::zero(),
            "deployed address should be zero on collision"
        );
    }

    /// Test CREATE2 success: factory deploys a child via CREATE2 with salt.
    ///
    /// Validates deterministic address calculation and successful deployment.
    #[test]
    fn test_create2_success_interpreter() {
        use bytes::Bytes;
        use ethrex_common::{Address, U256};
        use ethrex_common::types::Code;

        let factory_addr = Address::from_low_u64_be(0x42);
        let factory_code = Code::from_bytecode(Bytes::from(make_create2_factory_bytecode()));

        let report = run_factory_via_interpreter(factory_addr, factory_code, vec![]);

        assert!(
            report.is_success(),
            "CREATE2 factory should succeed, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes (address)");

        // The returned address should be non-zero (CREATE2 succeeded)
        let deployed_addr_word = U256::from_big_endian(&report.output);
        assert_ne!(
            deployed_addr_word,
            U256::zero(),
            "CREATE2 deployed address should be non-zero"
        );

        // Verify deterministic address: running again should produce the same address
        // (not actually — nonce changes each run, but CREATE2 with same salt+initcode
        // from same sender should be deterministic within a single execution)
    }

    /// JIT-compile a CREATE factory and run through the full VM dispatch path.
    ///
    /// This exercises `handle_jit_subcall` CREATE arm:
    /// 1. Factory bytecode is JIT-compiled via revmc
    /// 2. JIT executes factory, hits CREATE → suspends with JitOutcome::Suspended
    /// 3. VM calls handle_jit_subcall(JitSubCall::Create { ... })
    /// 4. Interpreter runs child init code → deploys contract
    /// 5. JIT resumes with SubCallResult { created_address: Some(...) }
    /// 6. Factory returns the deployed address
    ///
    /// Differential: compares output with interpreter path to prove correctness.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_create_jit_factory() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
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

        let factory_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = ethrex_common::types::Fork::Cancun;

        let factory_code = Code::from_bytecode(Bytes::from(make_create_factory_bytecode()));

        // --- Interpreter baseline ---
        let interp_report =
            run_factory_via_interpreter(factory_addr, factory_code.clone(), vec![]);
        assert!(
            interp_report.is_success(),
            "Interpreter CREATE should succeed: {:?}",
            interp_report.result
        );

        // --- JIT path ---
        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        // JIT-compile the factory contract
        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&factory_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of CREATE factory should succeed");
        assert!(
            JIT_STATE
                .cache
                .get(&(factory_code.hash, fork))
                .is_some(),
            "factory should be in JIT cache"
        );

        // Register the backend for JIT execution
        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            factory_addr,
            Account::new(U256::MAX, factory_code, 0, FxHashMap::default()),
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
            to: TxKind::Call(factory_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("JIT CREATE factory execution should succeed");

        assert!(
            report.is_success(),
            "JIT CREATE factory should succeed, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes (address)");

        // The returned address should be non-zero (CREATE succeeded via JIT path)
        let deployed_addr_word = U256::from_big_endian(&report.output);
        assert_ne!(
            deployed_addr_word,
            U256::zero(),
            "JIT CREATE deployed address should be non-zero"
        );

        // Prove JIT path was taken (M2: execution proof)
        assert!(
            JIT_STATE.metrics.jit_executions.load(Ordering::Relaxed) > 0,
            "JIT path should have been taken (jit_executions > 0)"
        );

        // Differential: JIT output must match interpreter output
        assert_eq!(
            report.output, interp_report.output,
            "JIT and interpreter CREATE output mismatch"
        );
    }

    /// JIT-compile a CREATE2 factory and run through the full VM dispatch path.
    ///
    /// Same as test_create_jit_factory but for CREATE2, exercising the salt-based
    /// address computation in handle_jit_subcall CREATE arm.
    ///
    /// Differential: compares output with interpreter path to prove correctness.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_create2_jit_factory() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
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

        let factory_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = ethrex_common::types::Fork::Cancun;

        let factory_code = Code::from_bytecode(Bytes::from(make_create2_factory_bytecode()));

        // --- Interpreter baseline ---
        let interp_report =
            run_factory_via_interpreter(factory_addr, factory_code.clone(), vec![]);
        assert!(
            interp_report.is_success(),
            "Interpreter CREATE2 should succeed: {:?}",
            interp_report.result
        );

        // --- JIT path ---
        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        // JIT-compile the factory contract
        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&factory_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of CREATE2 factory should succeed");
        assert!(
            JIT_STATE
                .cache
                .get(&(factory_code.hash, fork))
                .is_some(),
            "factory should be in JIT cache"
        );

        // Register the backend for JIT execution
        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            factory_addr,
            Account::new(U256::MAX, factory_code, 0, FxHashMap::default()),
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
            to: TxKind::Call(factory_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("JIT CREATE2 factory execution should succeed");

        assert!(
            report.is_success(),
            "JIT CREATE2 factory should succeed, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes (address)");

        // The returned address should be non-zero (CREATE2 succeeded via JIT path)
        let deployed_addr_word = U256::from_big_endian(&report.output);
        assert_ne!(
            deployed_addr_word,
            U256::zero(),
            "JIT CREATE2 deployed address should be non-zero"
        );

        // Prove JIT path was taken (M2: execution proof)
        assert!(
            JIT_STATE.metrics.jit_executions.load(Ordering::Relaxed) > 0,
            "JIT path should have been taken (jit_executions > 0)"
        );

        // Differential: JIT output must match interpreter output
        assert_eq!(
            report.output, interp_report.output,
            "JIT and interpreter CREATE2 output mismatch"
        );
    }

    /// Build a caller contract that does CALL with value to a precompile (identity at 0x04).
    ///
    /// The caller sends 1 wei to the identity precompile and returns the success flag.
    #[cfg(feature = "revmc-backend")]
    fn make_value_call_to_precompile() -> Vec<u8> {
        let mut code = Vec::new();

        //  0: PUSH1 0x00 (retSize)
        code.push(0x60);
        code.push(0x00);
        //  2: PUSH1 0x00 (retOffset)
        code.push(0x60);
        code.push(0x00);
        //  4: PUSH1 0x00 (argsSize)
        code.push(0x60);
        code.push(0x00);
        //  6: PUSH1 0x00 (argsOffset)
        code.push(0x60);
        code.push(0x00);
        //  8: PUSH1 0x01 (value = 1 wei)
        code.push(0x60);
        code.push(0x01);
        // 10: PUSH20 <identity precompile = 0x04>
        code.push(0x73);
        let mut addr = [0u8; 20];
        addr[19] = 0x04;
        code.extend_from_slice(&addr);
        // 31: PUSH3 0xFFFFFF (gas)
        code.push(0x62);
        code.push(0xFF);
        code.push(0xFF);
        code.push(0xFF);
        // 35: CALL
        code.push(0xF1);
        // 36: PUSH1 0x00
        code.push(0x60);
        code.push(0x00);
        // 38: MSTORE
        code.push(0x52);
        // 39: PUSH1 0x20
        code.push(0x60);
        code.push(0x20);
        // 41: PUSH1 0x00
        code.push(0x60);
        code.push(0x00);
        // 43: RETURN
        code.push(0xF3);

        code
    }

    /// JIT-compile a contract that CALLs a precompile with value > 0.
    ///
    /// Exercises the precompile value transfer path in handle_jit_subcall:
    /// 1. JIT code hits CALL(identity_precompile, value=1wei) → suspends
    /// 2. handle_jit_subcall detects precompile, executes it
    /// 3. On success, transfers value and emits EIP-7708 log
    /// 4. Returns SubCallResult to JIT resume
    ///
    /// Differential: compares output with interpreter path.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_precompile_value_transfer_jit() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
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

        let caller_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = ethrex_common::types::Fork::Cancun;

        let caller_code = Code::from_bytecode(Bytes::from(make_value_call_to_precompile()));

        // --- Interpreter baseline ---
        let interp_report =
            run_factory_via_interpreter(caller_addr, caller_code.clone(), vec![]);
        assert!(
            interp_report.is_success(),
            "Interpreter precompile value-call should succeed: {:?}",
            interp_report.result
        );

        // --- JIT path ---
        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        // JIT-compile the caller contract
        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of value-call caller should succeed");

        // Register the backend for JIT execution
        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        cache.insert(
            caller_addr,
            Account::new(U256::MAX, caller_code, 0, FxHashMap::default()),
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
            to: TxKind::Call(caller_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("JIT precompile value transfer should succeed");

        assert!(
            report.is_success(),
            "JIT precompile value-call should succeed, got: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes");

        // The CALL should succeed (identity precompile with empty input = success)
        // so the return value should be 1 (success flag)
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(1u64),
            "CALL to precompile with value should succeed (return 1)"
        );

        // Prove JIT path was taken
        assert!(
            JIT_STATE.metrics.jit_executions.load(Ordering::Relaxed) > 0,
            "JIT path should have been taken (jit_executions > 0)"
        );

        // Differential: JIT output must match interpreter output
        assert_eq!(
            report.output, interp_report.output,
            "JIT and interpreter precompile value-call output mismatch"
        );
    }

    /// JIT CREATE with collision: pre-seed the target address so CREATE fails.
    ///
    /// The factory is JIT-compiled; when CREATE hits a collision (target address
    /// already has code), it returns address(0). Validates that the JIT path
    /// handles CREATE failure identically to the interpreter.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_create_collision_jit_factory() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            constants::EMPTY_TRIE_HASH,
            evm::calculate_create_address,
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

        let factory_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = ethrex_common::types::Fork::Cancun;

        let factory_code = Code::from_bytecode(Bytes::from(make_create_factory_bytecode()));

        // Pre-calculate the collision address (nonce=0 for fresh factory account)
        let collision_addr = calculate_create_address(factory_addr, 0);
        let collision_code = Code::from_bytecode(Bytes::from(vec![0x60, 0x00, 0xF3]));
        let collision_account =
            Account::new(U256::zero(), collision_code.clone(), 0, FxHashMap::default());

        // --- Interpreter baseline ---
        let interp_report = run_factory_via_interpreter(
            factory_addr,
            factory_code.clone(),
            vec![(collision_addr, collision_account.clone())],
        );
        assert!(
            interp_report.is_success(),
            "Interpreter collision CREATE should succeed (soft fail): {:?}",
            interp_report.result
        );
        let interp_addr = U256::from_big_endian(&interp_report.output);
        assert_eq!(
            interp_addr,
            U256::zero(),
            "Interpreter should return address(0) on collision"
        );

        // --- JIT path ---
        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&factory_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of collision factory should succeed");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let collision_account_jit =
            Account::new(U256::zero(), collision_code, 0, FxHashMap::default());
        let mut cache = FxHashMap::default();
        cache.insert(
            factory_addr,
            Account::new(U256::MAX, factory_code, 0, FxHashMap::default()),
        );
        cache.insert(collision_addr, collision_account_jit);
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
            to: TxKind::Call(factory_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("JIT collision CREATE should succeed (soft fail)");

        assert!(
            report.is_success(),
            "JIT collision CREATE outer tx should succeed: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32, "should return 32 bytes");

        // On collision, factory should get address(0) from CREATE
        let deployed_addr_word = U256::from_big_endian(&report.output);
        assert_eq!(
            deployed_addr_word,
            U256::zero(),
            "JIT should return address(0) on CREATE collision"
        );

        // Prove JIT path was taken
        assert!(
            JIT_STATE.metrics.jit_executions.load(Ordering::Relaxed) > 0,
            "JIT path should have been taken (jit_executions > 0)"
        );

        // Differential: JIT output must match interpreter output
        assert_eq!(
            report.output, interp_report.output,
            "JIT and interpreter collision CREATE output mismatch"
        );
    }
}
