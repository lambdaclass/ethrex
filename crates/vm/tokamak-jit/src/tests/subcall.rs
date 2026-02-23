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
}
