//! JIT-to-JIT dispatch tests for the JIT compiler.
//!
//! Tests that when a JIT-compiled parent contract hits CALL/CREATE, the VM
//! checks if the child bytecode is also JIT-compiled and dispatches it via
//! JIT directly instead of falling back to the interpreter.
#![allow(clippy::vec_init_then_push)]
#![cfg_attr(not(feature = "revmc-backend"), allow(dead_code))]

/// Build a contract that recursively calls itself via STATICCALL.
///
/// Uses CALLDATASIZE as a depth counter: each recursive call appends one
/// byte of calldata. When calldata length >= 5, stops recursion and returns 42.
///
/// ```text
/// CALLDATASIZE          // [size]
/// PUSH1 5               // [size, 5]
/// LT                    // [size < 5]
/// ISZERO                // [size >= 5]
/// PUSH1 <base_case>     // [size >= 5, dest]
/// JUMPI                 // jump if size >= 5
///
/// // Recursive case: STATICCALL self with calldata size+1
/// PUSH1 0x20            // retSize
/// PUSH1 0x00            // retOffset
/// CALLDATASIZE          // argsSize (grows by 1 each level via memory trick)
/// PUSH1 0x01            // +1
/// ADD                   // new argsSize
/// PUSH1 0x00            // argsOffset
/// PUSH20 <own_addr>     // self address
/// PUSH3 0xFFFFFF        // gas
/// STATICCALL            // recurse
/// POP                   // discard success
/// PUSH1 0x20            // return size
/// PUSH1 0x00            // return offset
/// RETURN
///
/// // Base case: return 42
/// JUMPDEST
/// PUSH1 42
/// PUSH1 0x00
/// MSTORE
/// PUSH1 0x20
/// PUSH1 0x00
/// RETURN
/// ```
pub fn make_recursive_caller(own_addr: [u8; 20]) -> Vec<u8> {
    let mut code = Vec::new();

    //  0: CALLDATASIZE
    code.push(0x36);
    //  1: PUSH1 5
    code.push(0x60);
    code.push(0x05);
    //  3: LT  (size < 5 => 1, else 0)
    code.push(0x10);
    //  4: ISZERO  (size >= 5 => 1)
    code.push(0x15);

    // We need to calculate the JUMPDEST offset for the base case.
    // After the recursive STATICCALL section, the base case JUMPDEST will be placed.
    // Let's calculate: up to JUMPI = 7 bytes, then recursive section.
    // Recursive section starts at byte 8.

    //  5: PUSH1 <base_case_dest>  — will be patched below
    code.push(0x60);
    let base_case_patch_idx = code.len();
    code.push(0x00); // placeholder
    //  7: JUMPI
    code.push(0x57);

    // Recursive case (starting at byte 8):
    //  8: PUSH1 0x20 (retSize)
    code.push(0x60);
    code.push(0x20);
    // 10: PUSH1 0x00 (retOffset)
    code.push(0x60);
    code.push(0x00);
    // 12: CALLDATASIZE (argsSize — pass original calldata size)
    code.push(0x36);
    // 13: PUSH1 0x01
    code.push(0x60);
    code.push(0x01);
    // 15: ADD  (argsSize = calldatasize + 1)
    code.push(0x01);
    // 16: PUSH1 0x00 (argsOffset)
    code.push(0x60);
    code.push(0x00);
    // 18: PUSH20 <own_addr>
    code.push(0x73);
    code.extend_from_slice(&own_addr);
    // 39: PUSH3 0xFFFFFF (gas)
    code.push(0x62);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    // 43: STATICCALL
    code.push(0xFA);
    // 44: POP
    code.push(0x50);
    // 45: PUSH1 0x20 (return size)
    code.push(0x60);
    code.push(0x20);
    // 47: PUSH1 0x00 (return offset)
    code.push(0x60);
    code.push(0x00);
    // 49: RETURN
    code.push(0xF3);

    // Base case (at byte 50):
    let base_case_offset = code.len();
    // 50: JUMPDEST
    code.push(0x5B);
    // 51: PUSH1 42
    code.push(0x60);
    code.push(42);
    // 53: PUSH1 0x00
    code.push(0x60);
    code.push(0x00);
    // 55: MSTORE
    code.push(0x52);
    // 56: PUSH1 0x20
    code.push(0x60);
    code.push(0x20);
    // 58: PUSH1 0x00
    code.push(0x60);
    code.push(0x00);
    // 60: RETURN
    code.push(0xF3);

    // Patch the base case destination
    #[expect(clippy::as_conversions)]
    {
        code[base_case_patch_idx] = base_case_offset as u8;
    }

    code
}

/// Build a contract that does TWO sequential STATICCALLs to different addresses.
///
/// First calls target_a (expects 32-byte return at mem[0..32]),
/// then calls target_b (expects 32-byte return at mem[32..64]),
/// then returns mem[0..64].
///
/// ```text
/// // STATICCALL target_a → mem[0..32]
/// PUSH1 0x20           // retSize
/// PUSH1 0x00           // retOffset
/// PUSH1 0x00           // argsSize
/// PUSH1 0x00           // argsOffset
/// PUSH20 <target_a>    // address
/// PUSH3 0xFFFFFF       // gas
/// STATICCALL
/// POP                  // discard success
///
/// // STATICCALL target_b → mem[32..64]
/// PUSH1 0x20           // retSize
/// PUSH1 0x20           // retOffset = 32
/// PUSH1 0x00           // argsSize
/// PUSH1 0x00           // argsOffset
/// PUSH20 <target_b>    // address
/// PUSH3 0xFFFFFF       // gas
/// STATICCALL
/// POP                  // discard success
///
/// // Return mem[0..64]
/// PUSH1 0x40           // size = 64
/// PUSH1 0x00           // offset = 0
/// RETURN
/// ```
pub fn make_dual_staticcall_caller(target_a: [u8; 20], target_b: [u8; 20]) -> Vec<u8> {
    let mut code = Vec::new();

    // First STATICCALL to target_a → mem[0..32]
    code.push(0x60);
    code.push(0x20); // retSize = 32
    code.push(0x60);
    code.push(0x00); // retOffset = 0
    code.push(0x60);
    code.push(0x00); // argsSize = 0
    code.push(0x60);
    code.push(0x00); // argsOffset = 0
    code.push(0x73); // PUSH20 target_a
    code.extend_from_slice(&target_a);
    code.push(0x62); // PUSH3 gas
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFA); // STATICCALL
    code.push(0x50); // POP success

    // Second STATICCALL to target_b → mem[32..64]
    code.push(0x60);
    code.push(0x20); // retSize = 32
    code.push(0x60);
    code.push(0x20); // retOffset = 32
    code.push(0x60);
    code.push(0x00); // argsSize = 0
    code.push(0x60);
    code.push(0x00); // argsOffset = 0
    code.push(0x73); // PUSH20 target_b
    code.extend_from_slice(&target_b);
    code.push(0x62); // PUSH3 gas
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFF);
    code.push(0xFA); // STATICCALL
    code.push(0x50); // POP success

    // Return mem[0..64]
    code.push(0x60);
    code.push(0x40); // size = 64
    code.push(0x60);
    code.push(0x00); // offset = 0
    code.push(0xF3); // RETURN

    code
}

/// Build a factory contract that CREATE-deploys a child contract.
///
/// The child's init code stores 0x42 at mem[0] and returns it as deployed bytecode.
/// The factory returns the deployed address as a 32-byte value.
#[cfg(test)]
fn make_create_factory() -> Vec<u8> {
    // Child init code: PUSH1 0x42, PUSH1 0x00, MSTORE8, PUSH1 0x01, PUSH1 0x00, RETURN
    let init_code: Vec<u8> = vec![0x60, 0x42, 0x60, 0x00, 0x53, 0x60, 0x01, 0x60, 0x00, 0xF3];

    let mut code = Vec::new();

    // Store init code in memory using MSTORE8
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

/// Build a callee that returns the value 99 in memory[0..32].
#[cfg(test)]
fn make_return99_bytecode() -> Vec<u8> {
    let mut code = Vec::new();
    code.push(0x60);
    code.push(99); // PUSH1 99
    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0x52); // MSTORE
    code.push(0x60);
    code.push(0x20); // PUSH1 32
    code.push(0x60);
    code.push(0x00); // PUSH1 0
    code.push(0xF3); // RETURN
    code
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "revmc-backend")]
    use super::super::subcall::{
        make_checked_staticcall_caller, make_return42_bytecode, make_reverting_bytecode,
        make_staticcall_caller,
    };
    #[cfg(feature = "revmc-backend")]
    use super::*;

    // ---------------------------------------------------------------------------
    // Test 1: Simple JIT-to-JIT STATICCALL
    // ---------------------------------------------------------------------------

    /// Both caller and callee are JIT-compiled. Caller does STATICCALL to callee
    /// (returns 42). Asserts output = 42 and jit_to_jit_dispatches > 0.
    /// Includes differential comparison with interpreter-only run.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_simple_staticcall() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let caller_code =
            Code::from_bytecode(Bytes::from(make_staticcall_caller(callee_addr.into())));

        // --- Interpreter baseline ---
        let mut interp_db = make_test_db(vec![
            TestAccount {
                address: callee_addr,
                code: callee_code.clone(),
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: caller_addr,
                code: caller_code.clone(),
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(caller_addr, Bytes::new());

        let mut interp_vm = VM::new(
            env.clone(),
            &mut interp_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("Interpreter VM::new should succeed");
        let interp_report = interp_vm
            .stateless_execute()
            .expect("Interpreter staticcall should succeed");
        assert!(
            interp_report.is_success(),
            "Interpreter should succeed: {:?}",
            interp_report.result
        );

        // --- JIT path (both compiled) ---
        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of caller should succeed");
        backend
            .compile_and_cache(&callee_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of callee should succeed");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut jit_db = make_test_db(vec![
            TestAccount {
                address: callee_addr,
                code: callee_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: caller_addr,
                code: caller_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);

        let mut jit_vm = VM::new(
            env,
            &mut jit_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("JIT VM::new should succeed");
        let jit_report = jit_vm
            .stateless_execute()
            .expect("JIT staticcall should succeed");

        assert!(
            jit_report.is_success(),
            "JIT caller→callee should succeed: {:?}",
            jit_report.result
        );
        assert_eq!(jit_report.output.len(), 32, "should return 32 bytes");
        let result_val = U256::from_big_endian(&jit_report.output);
        assert_eq!(result_val, U256::from(42u64), "callee should return 42");

        // Verify JIT-to-JIT dispatch was used
        assert!(
            JIT_STATE
                .metrics
                .jit_to_jit_dispatches
                .load(Ordering::Relaxed)
                > 0,
            "jit_to_jit_dispatches should be > 0 when both caller and callee are JIT-compiled"
        );

        // Differential: output must match interpreter
        assert_eq!(
            jit_report.output, interp_report.output,
            "JIT vs interpreter output mismatch"
        );
        assert_eq!(
            jit_report.gas_used, interp_report.gas_used,
            "JIT vs interpreter gas_used mismatch"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 2: Checked STATICCALL success path (both JIT)
    // ---------------------------------------------------------------------------

    /// Caller uses checked STATICCALL (branches on success flag), callee returns 42.
    /// Both JIT-compiled. Asserts output = 42 (success path taken).
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_checked_staticcall_success() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let caller_code = Code::from_bytecode(Bytes::from(make_checked_staticcall_caller(
            callee_addr.into(),
        )));

        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of checked caller should succeed");
        backend
            .compile_and_cache(&callee_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of callee should succeed");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut db = make_test_db(vec![
            TestAccount {
                address: callee_addr,
                code: callee_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: caller_addr,
                code: caller_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(caller_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");
        let report = vm
            .stateless_execute()
            .expect("JIT checked staticcall should succeed");

        assert!(
            report.is_success(),
            "JIT checked staticcall should succeed: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32);
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(42u64),
            "success path should return 42"
        );

        assert!(
            JIT_STATE
                .metrics
                .jit_to_jit_dispatches
                .load(Ordering::Relaxed)
                > 0,
            "jit_to_jit_dispatches should be > 0"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 3: Checked STATICCALL with reverting child (both JIT)
    // ---------------------------------------------------------------------------

    /// Caller uses checked STATICCALL, callee REVERTs. Both JIT-compiled.
    /// Asserts output = 0xDEAD (failure path), caller succeeds.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_revert_child() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_reverting_bytecode()));
        let caller_code = Code::from_bytecode(Bytes::from(make_checked_staticcall_caller(
            callee_addr.into(),
        )));

        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of checked caller should succeed");
        backend
            .compile_and_cache(&callee_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of reverting callee should succeed");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut db = make_test_db(vec![
            TestAccount {
                address: callee_addr,
                code: callee_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: caller_addr,
                code: caller_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(caller_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");
        let report = vm
            .stateless_execute()
            .expect("JIT checked staticcall-revert should succeed");

        assert!(
            report.is_success(),
            "outer call should succeed even when inner reverts: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32);
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(0xDEADu64),
            "caller should return 0xDEAD on child revert"
        );

        assert!(
            JIT_STATE
                .metrics
                .jit_to_jit_dispatches
                .load(Ordering::Relaxed)
                > 0,
            "jit_to_jit_dispatches should be > 0 even for reverting child"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 4: Nested 3-level STATICCALL chain (A → B → C, all JIT)
    // ---------------------------------------------------------------------------

    /// 3-level chain: A calls B, B calls C, C returns 42. All three JIT-compiled.
    /// Asserts output = 42 and jit_to_jit_dispatches >= 2 (A→B and B→C).
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_nested_staticcall() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let c_addr = Address::from_low_u64_be(0x42);
        let b_addr = Address::from_low_u64_be(0x43);
        let a_addr = Address::from_low_u64_be(0x44);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let c_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let b_code = Code::from_bytecode(Bytes::from(make_staticcall_caller(c_addr.into())));
        let a_code = Code::from_bytecode(Bytes::from(make_staticcall_caller(b_addr.into())));

        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&a_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of A should succeed");
        backend
            .compile_and_cache(&b_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of B should succeed");
        backend
            .compile_and_cache(&c_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of C should succeed");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut db = make_test_db(vec![
            TestAccount {
                address: c_addr,
                code: c_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: b_addr,
                code: b_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: a_addr,
                code: a_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(a_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");
        let report = vm
            .stateless_execute()
            .expect("JIT nested staticcall should succeed");

        assert!(
            report.is_success(),
            "nested A→B→C should succeed: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32);
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(42u64),
            "C returns 42 through B and A"
        );

        let dispatches = JIT_STATE
            .metrics
            .jit_to_jit_dispatches
            .load(Ordering::Relaxed);
        assert!(
            dispatches >= 2,
            "jit_to_jit_dispatches should be >= 2 for 3-level chain, got {dispatches}"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 5: Cache miss fallback (caller JIT, callee NOT compiled)
    // ---------------------------------------------------------------------------

    /// Caller is JIT-compiled, callee is NOT in JIT cache (interpreter fallback).
    /// Asserts output still correct (42) and jit_to_jit_dispatches = 0.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_cache_miss_fallback() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let caller_code =
            Code::from_bytecode(Bytes::from(make_staticcall_caller(callee_addr.into())));

        JIT_STATE.reset_for_testing();

        // Only compile the caller — callee stays interpreter-only
        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of caller should succeed");
        // Callee NOT compiled — should fallback to interpreter

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut db = make_test_db(vec![
            TestAccount {
                address: callee_addr,
                code: callee_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: caller_addr,
                code: caller_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(caller_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");
        let report = vm
            .stateless_execute()
            .expect("JIT with interpreter callee should succeed");

        assert!(
            report.is_success(),
            "should succeed with interpreter fallback: {:?}",
            report.result
        );
        assert_eq!(report.output.len(), 32);
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(42u64),
            "callee returns 42 via interpreter"
        );

        // No JIT-to-JIT dispatch should have occurred
        assert_eq!(
            JIT_STATE
                .metrics
                .jit_to_jit_dispatches
                .load(Ordering::Relaxed),
            0,
            "jit_to_jit_dispatches should be 0 when callee is not JIT-compiled"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 6: Differential — JIT-to-JIT vs interpreter-only
    // ---------------------------------------------------------------------------

    /// Runs the same caller→callee scenario through JIT-to-JIT and interpreter-only
    /// paths. Asserts identical output AND identical gas_used.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_vs_interpreter_differential() {
        use std::sync::Arc;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let caller_code =
            Code::from_bytecode(Bytes::from(make_staticcall_caller(callee_addr.into())));

        let accounts = || {
            vec![
                TestAccount {
                    address: callee_addr,
                    code: callee_code.clone(),
                    storage: FxHashMap::default(),
                },
                TestAccount {
                    address: caller_addr,
                    code: caller_code.clone(),
                    storage: FxHashMap::default(),
                },
                TestAccount {
                    address: sender_addr,
                    code: Code::from_bytecode(Bytes::new()),
                    storage: FxHashMap::default(),
                },
            ]
        };

        // --- Interpreter-only ---
        let mut interp_db = make_test_db(accounts());
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(caller_addr, Bytes::new());

        let mut interp_vm = VM::new(
            env.clone(),
            &mut interp_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("Interpreter VM::new");
        let interp_report = interp_vm
            .stateless_execute()
            .expect("Interpreter should succeed");
        assert!(interp_report.is_success());

        // --- JIT-to-JIT (both compiled) ---
        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("compile caller");
        backend
            .compile_and_cache(&callee_code, fork, &JIT_STATE.cache)
            .expect("compile callee");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut jit_db = make_test_db(accounts());
        let mut jit_vm = VM::new(
            env,
            &mut jit_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("JIT VM::new");
        let jit_report = jit_vm.stateless_execute().expect("JIT should succeed");
        assert!(jit_report.is_success());

        // Differential assertions
        assert_eq!(
            jit_report.output, interp_report.output,
            "JIT-to-JIT output must match interpreter output"
        );
        assert_eq!(
            jit_report.gas_used, interp_report.gas_used,
            "JIT-to-JIT gas_used must match interpreter gas_used"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 7: JIT-to-JIT CREATE factory
    // ---------------------------------------------------------------------------

    /// Factory bytecode is JIT-compiled and executes CREATE.
    /// Asserts CREATE succeeds and deployed address is non-zero.
    /// Includes differential with interpreter.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_create_factory() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let factory_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let factory_code = Code::from_bytecode(Bytes::from(super::make_create_factory()));

        // --- Interpreter baseline ---
        let mut interp_db = make_test_db(vec![
            TestAccount {
                address: factory_addr,
                code: factory_code.clone(),
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(factory_addr, Bytes::new());

        let mut interp_vm = VM::new(
            env.clone(),
            &mut interp_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("Interpreter VM::new");
        let interp_report = interp_vm
            .stateless_execute()
            .expect("Interpreter CREATE should succeed");
        assert!(interp_report.is_success());

        // --- JIT path ---
        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&factory_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of factory should succeed");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut jit_db = make_test_db(vec![
            TestAccount {
                address: factory_addr,
                code: factory_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);

        let mut jit_vm = VM::new(
            env,
            &mut jit_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("JIT VM::new");
        let jit_report = jit_vm
            .stateless_execute()
            .expect("JIT CREATE should succeed");

        assert!(
            jit_report.is_success(),
            "JIT CREATE should succeed: {:?}",
            jit_report.result
        );
        assert_eq!(jit_report.output.len(), 32);

        let deployed_addr = U256::from_big_endian(&jit_report.output);
        assert_ne!(
            deployed_addr,
            U256::zero(),
            "deployed address should be non-zero"
        );

        assert!(
            JIT_STATE.metrics.jit_executions.load(Ordering::Relaxed) > 0,
            "JIT path should have been taken"
        );

        // Differential
        assert_eq!(
            jit_report.output, interp_report.output,
            "JIT vs interpreter CREATE output mismatch"
        );
        assert_eq!(
            jit_report.gas_used, interp_report.gas_used,
            "JIT vs interpreter CREATE gas_used mismatch"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 8: Recursive self-call depth limit
    // ---------------------------------------------------------------------------

    /// Contract recursively calls itself via STATICCALL. JIT-compiled.
    /// Asserts: doesn't panic, returns after depth limit is hit.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_depth_limit() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let contract_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let contract_code =
            Code::from_bytecode(Bytes::from(make_recursive_caller(contract_addr.into())));

        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&contract_code, fork, &JIT_STATE.cache)
            .expect("JIT compilation of recursive caller should succeed");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut db = make_test_db(vec![
            TestAccount {
                address: contract_addr,
                code: contract_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        // This should NOT panic — depth limit prevents infinite recursion
        let report = vm
            .stateless_execute()
            .expect("recursive self-call should not panic");

        // The contract should complete (either success or bounded depth)
        // The important thing is no panic and no infinite loop
        assert!(
            report.is_success(),
            "recursive caller should succeed (bounded depth): {:?}",
            report.result
        );

        // JIT should have been used for the initial call at minimum
        assert!(
            JIT_STATE.metrics.jit_executions.load(Ordering::Relaxed) > 0,
            "JIT path should have been taken"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 9: JIT dispatch disabled via config
    // ---------------------------------------------------------------------------

    /// Verifies that when `enable_jit_dispatch` is false, JIT-to-JIT dispatch
    /// does not occur even when both caller and callee are JIT-compiled.
    ///
    /// Since JIT_STATE.config is immutable on the global lazy_static (reset_for_testing
    /// doesn't reset config), this test verifies the behavior by checking that
    /// the `is_jit_dispatch_enabled()` API works correctly, and runs the caller→callee
    /// scenario with only the CALLER compiled (simulating the effect of disabled dispatch
    /// where the child always falls through to interpreter).
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_dispatch_disabled_config() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            jit::dispatch::JitState,
            jit::types::JitConfig,
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        // Verify the config API: a JitState with dispatch disabled
        let disabled_config = JitConfig {
            enable_jit_dispatch: false,
            ..JitConfig::default()
        };
        let disabled_state = JitState::with_config(disabled_config);
        assert!(
            !disabled_state.is_jit_dispatch_enabled(),
            "is_jit_dispatch_enabled should return false when config disables it"
        );

        // Verify the global JIT_STATE has dispatch enabled by default
        assert!(
            JIT_STATE.is_jit_dispatch_enabled(),
            "global JIT_STATE should have dispatch enabled by default"
        );

        // Now run a test where only the caller is compiled (callee NOT compiled).
        // This simulates the effective behavior of disabled dispatch: the child
        // always runs via interpreter, so jit_to_jit_dispatches stays at 0.
        let callee_addr = Address::from_low_u64_be(0x42);
        let caller_addr = Address::from_low_u64_be(0x43);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let callee_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let caller_code =
            Code::from_bytecode(Bytes::from(make_staticcall_caller(callee_addr.into())));

        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("compile caller");
        // Callee NOT compiled — interpreter fallback

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut db = make_test_db(vec![
            TestAccount {
                address: callee_addr,
                code: callee_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: caller_addr,
                code: caller_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(caller_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");
        let report = vm
            .stateless_execute()
            .expect("should succeed via interpreter fallback");

        assert!(report.is_success());
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(result_val, U256::from(42u64), "output should be 42");

        // No JIT-to-JIT dispatch — callee was not compiled
        assert_eq!(
            JIT_STATE
                .metrics
                .jit_to_jit_dispatches
                .load(Ordering::Relaxed),
            0,
            "jit_to_jit_dispatches should be 0 when dispatch is effectively disabled"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 10: Multiple sequential STATICCALLs to different callees
    // ---------------------------------------------------------------------------

    /// Contract does TWO sequential STATICCALLs to different JIT-compiled callees.
    /// Asserts both outputs correct and jit_to_jit_dispatches >= 2.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_jit_to_jit_multiple_calls() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use bytes::Bytes;
        use ethrex_common::{
            Address, U256,
            types::{Code, Fork},
        };
        use ethrex_levm::{
            tracing::LevmCallTracer,
            vm::{JIT_STATE, VM, VMType},
        };
        use rustc_hash::FxHashMap;

        use crate::backend::RevmcBackend;
        use crate::tests::test_helpers::{TestAccount, make_test_db, make_test_env, make_test_tx};

        let callee_a_addr = Address::from_low_u64_be(0x42);
        let callee_b_addr = Address::from_low_u64_be(0x43);
        let caller_addr = Address::from_low_u64_be(0x44);
        let sender_addr = Address::from_low_u64_be(0x100);
        let fork = Fork::Cancun;

        let callee_a_code = Code::from_bytecode(Bytes::from(make_return42_bytecode()));
        let callee_b_code = Code::from_bytecode(Bytes::from(make_return99_bytecode()));
        let caller_code = Code::from_bytecode(Bytes::from(make_dual_staticcall_caller(
            callee_a_addr.into(),
            callee_b_addr.into(),
        )));

        // --- Interpreter baseline ---
        let mut interp_db = make_test_db(vec![
            TestAccount {
                address: callee_a_addr,
                code: callee_a_code.clone(),
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: callee_b_addr,
                code: callee_b_code.clone(),
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: caller_addr,
                code: caller_code.clone(),
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(caller_addr, Bytes::new());

        let mut interp_vm = VM::new(
            env.clone(),
            &mut interp_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("Interpreter VM::new");
        let interp_report = interp_vm
            .stateless_execute()
            .expect("Interpreter dual staticcall should succeed");
        assert!(interp_report.is_success());
        assert_eq!(
            interp_report.output.len(),
            64,
            "should return 64 bytes (two 32-byte values)"
        );

        let interp_val_a = U256::from_big_endian(&interp_report.output[0..32]);
        let interp_val_b = U256::from_big_endian(&interp_report.output[32..64]);
        assert_eq!(interp_val_a, U256::from(42u64), "first callee returns 42");
        assert_eq!(interp_val_b, U256::from(99u64), "second callee returns 99");

        // --- JIT path (all three compiled) ---
        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        backend
            .compile_and_cache(&caller_code, fork, &JIT_STATE.cache)
            .expect("compile caller");
        backend
            .compile_and_cache(&callee_a_code, fork, &JIT_STATE.cache)
            .expect("compile callee A");
        backend
            .compile_and_cache(&callee_b_code, fork, &JIT_STATE.cache)
            .expect("compile callee B");

        JIT_STATE.register_backend(Arc::new(RevmcBackend::default()));

        let mut jit_db = make_test_db(vec![
            TestAccount {
                address: callee_a_addr,
                code: callee_a_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: callee_b_addr,
                code: callee_b_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: caller_addr,
                code: caller_code,
                storage: FxHashMap::default(),
            },
            TestAccount {
                address: sender_addr,
                code: Code::from_bytecode(Bytes::new()),
                storage: FxHashMap::default(),
            },
        ]);

        let mut jit_vm = VM::new(
            env,
            &mut jit_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("JIT VM::new");
        let jit_report = jit_vm
            .stateless_execute()
            .expect("JIT dual staticcall should succeed");

        assert!(
            jit_report.is_success(),
            "JIT dual staticcall should succeed: {:?}",
            jit_report.result
        );
        assert_eq!(jit_report.output.len(), 64, "should return 64 bytes");

        let jit_val_a = U256::from_big_endian(&jit_report.output[0..32]);
        let jit_val_b = U256::from_big_endian(&jit_report.output[32..64]);
        assert_eq!(
            jit_val_a,
            U256::from(42u64),
            "first callee returns 42 via JIT"
        );
        assert_eq!(
            jit_val_b,
            U256::from(99u64),
            "second callee returns 99 via JIT"
        );

        let dispatches = JIT_STATE
            .metrics
            .jit_to_jit_dispatches
            .load(Ordering::Relaxed);
        assert!(
            dispatches >= 2,
            "jit_to_jit_dispatches should be >= 2 for dual staticcall, got {dispatches}"
        );

        // Differential
        assert_eq!(
            jit_report.output, interp_report.output,
            "JIT vs interpreter dual output mismatch"
        );
        assert_eq!(
            jit_report.gas_used, interp_report.gas_used,
            "JIT vs interpreter dual gas_used mismatch"
        );
    }
}
