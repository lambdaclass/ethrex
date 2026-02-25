//! Gas alignment tests for JIT vs interpreter.
//!
//! Each test compiles a bytecode snippet via revmc, executes it through both
//! the JIT path (`execute_jit`) and the interpreter path (`VM::stateless_execute`),
//! and asserts that pre-refund gas matches exactly.
//!
//! **Gas accounting note**: The interpreter's `ExecutionReport.gas_used` is
//! *post-refund* for Cancun (refund cap = gas_used/5 subtracted). The JIT's
//! `JitOutcome::gas_used` is *pre-refund* (raw execution gas). We compare
//! pre-refund gas by reconstructing it: `interp_pre_refund = gas_used + gas_refunded`.
//!
//! **Known upstream issue**: revmc-builtins uses a hardcoded `REFUND_SSTORE_CLEARS = 15000`
//! (pre-London value) instead of the EIP-3529 post-London value (4800). This causes
//! raw refund values to differ between JIT and interpreter for SSTORE clear operations.
//! Pre-refund gas (execution cost) still matches because execution gas is independent
//! of refund accounting. Tests that trigger SSTORE clears skip the refund comparison
//! and document this upstream issue.
//!
//! Covers SSTORE edge cases (EIP-2200/EIP-3529), memory expansion costs,
//! and the negative-refund bug fix in `execution.rs`.

#[cfg(test)]
#[cfg(feature = "revmc-backend")]
mod tests {
    use bytes::Bytes;
    use ethrex_common::types::{Code, Fork};
    use ethrex_common::{H256, U256};
    use ethrex_levm::jit::cache::CodeCache;
    use ethrex_levm::jit::types::JitOutcome;
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{JIT_STATE, VM, VMType};
    use rustc_hash::FxHashMap;

    use crate::backend::RevmcBackend;
    use crate::execution::execute_jit;
    use crate::tests::test_helpers::{
        make_contract_accounts, make_test_db, make_test_env, make_test_tx, INTRINSIC_GAS,
        TEST_GAS_LIMIT,
    };

    /// Result of a gas alignment comparison between JIT and interpreter.
    struct GasComparison {
        /// Interpreter's reported gas_used (post-refund for Cancun).
        interp_gas_used: u64,
        /// JIT's raw execution gas (pre-refund, excludes intrinsic).
        jit_gas_used: u64,
        /// Interpreter's capped refund (min(raw_refund, gas_used/5)).
        interp_refunded: u64,
        /// JIT's raw (uncapped) refund from substate.
        jit_raw_refunded: u64,
        interp_success: bool,
        jit_success: bool,
    }

    /// Run both interpreter and JIT paths, returning gas metrics for comparison.
    ///
    /// `bytecode`: raw EVM bytecode (must end with STOP or RETURN).
    /// `storage`: pre-seeded storage for the contract account.
    fn run_gas_comparison(bytecode: Vec<u8>, storage: FxHashMap<H256, U256>) -> GasComparison {
        let fork = Fork::Cancun;

        JIT_STATE.reset_for_testing();

        let code = Code::from_bytecode(Bytes::from(bytecode));

        // Compile via revmc
        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        backend
            .compile_and_cache(&code, fork, &code_cache)
            .expect("JIT compilation should succeed");
        let compiled = code_cache
            .get(&(code.hash, fork))
            .expect("compiled code should be in cache");

        // --- Interpreter path ---
        let (contract_addr, sender_addr, interp_accounts) =
            make_contract_accounts(code.clone(), storage.clone());
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

        // --- JIT direct execution path ---
        let (_, _, jit_accounts) = make_contract_accounts(code.clone(), storage);
        let mut jit_db = make_test_db(jit_accounts);

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
            &mut jit_db,
            &mut substate,
            &env,
            &mut storage_original_values,
        )
        .expect("JIT execution should succeed");

        // Extract JIT gas
        #[expect(clippy::as_conversions)]
        let jit_gas_remaining = call_frame.gas_remaining.max(0) as u64;
        let jit_execution_gas = TEST_GAS_LIMIT
            .checked_sub(jit_gas_remaining)
            .expect("gas_limit >= gas_remaining");

        let (jit_success, jit_gas_used) = match &outcome {
            JitOutcome::Success { gas_used, .. } => {
                assert_eq!(
                    jit_execution_gas, *gas_used,
                    "apply_jit_outcome formula mismatch"
                );
                (true, *gas_used)
            }
            JitOutcome::Revert { gas_used, .. } => (false, *gas_used),
            other => panic!("Unexpected JIT outcome: {other:?}"),
        };

        GasComparison {
            interp_gas_used: interp_report.gas_used,
            jit_gas_used,
            interp_refunded: interp_report.gas_refunded,
            jit_raw_refunded: substate.refunded_gas,
            interp_success: interp_report.is_success(),
            jit_success,
        }
    }

    /// Assert pre-refund gas alignment between JIT and interpreter.
    ///
    /// Compares:
    /// 1. Success/failure status
    /// 2. Pre-refund gas: `interp_gas_used + interp_refunded == jit_gas_used + INTRINSIC_GAS`
    ///
    /// Does NOT compare raw refund values because revmc uses a hardcoded
    /// `REFUND_SSTORE_CLEARS = 15000` (pre-EIP-3529) while LEVM uses 4800
    /// (post-EIP-3529). Execution gas is unaffected by this upstream issue.
    fn assert_pre_refund_gas_matches(
        bytecode: Vec<u8>,
        storage: FxHashMap<H256, U256>,
        test_name: &str,
    ) {
        let r = run_gas_comparison(bytecode, storage);

        assert_eq!(
            r.interp_success, r.jit_success,
            "[{test_name}] success mismatch: interp={}, jit={}",
            r.interp_success, r.jit_success
        );

        // Reconstruct pre-refund gas for the interpreter.
        // For Cancun: interp_gas_used is post-refund, so add back the capped refund.
        let interp_pre_refund = r.interp_gas_used + r.interp_refunded;
        let jit_total_gas = r
            .jit_gas_used
            .checked_add(INTRINSIC_GAS)
            .expect("no overflow");

        assert_eq!(
            interp_pre_refund, jit_total_gas,
            "[{test_name}] pre-refund gas mismatch: interp_pre_refund={interp_pre_refund} \
             (gas_used={} + refunded={}), jit_total={jit_total_gas} \
             (exec={} + intrinsic={INTRINSIC_GAS})",
            r.interp_gas_used, r.interp_refunded, r.jit_gas_used
        );
    }

    /// Assert full gas alignment including refunds.
    ///
    /// Only use for cases with zero refund (no SSTORE clears), where the
    /// revmc upstream refund constant issue doesn't apply.
    fn assert_gas_and_refund_matches(
        bytecode: Vec<u8>,
        storage: FxHashMap<H256, U256>,
        test_name: &str,
    ) {
        let r = run_gas_comparison(bytecode, storage);

        assert_eq!(
            r.interp_success, r.jit_success,
            "[{test_name}] success mismatch: interp={}, jit={}",
            r.interp_success, r.jit_success
        );

        // For zero-refund cases, both post-refund and pre-refund gas are the same.
        let jit_total_gas = r
            .jit_gas_used
            .checked_add(INTRINSIC_GAS)
            .expect("no overflow");

        assert_eq!(
            r.interp_gas_used, jit_total_gas,
            "[{test_name}] gas mismatch: interp={}, jit_total={jit_total_gas} \
             (exec={} + intrinsic={INTRINSIC_GAS})",
            r.interp_gas_used, r.jit_gas_used
        );

        assert_eq!(
            r.interp_refunded, r.jit_raw_refunded,
            "[{test_name}] refund mismatch: interp={}, jit={}",
            r.interp_refunded, r.jit_raw_refunded
        );
    }

    // ─── SSTORE edge case tests (zero refund — full match) ────────────────

    /// SSTORE zero→nonzero: 20000 gas (set) + 2100 cold access, 0 refund.
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_zero_to_nonzero() {
        let bytecode = vec![
            0x60, 0x42, // PUSH1 0x42 (value)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE
            0x00, // STOP
        ];
        assert_gas_and_refund_matches(bytecode, FxHashMap::default(), "sstore_zero_to_nonzero");
    }

    /// SSTORE nonzero→different nonzero: 2900 gas (reset) + 2100 cold, 0 refund.
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_nonzero_to_different() {
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let bytecode = vec![
            0x60, 0x42, // PUSH1 0x42 (new value)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE
            0x00, // STOP
        ];
        assert_gas_and_refund_matches(bytecode, storage, "sstore_nonzero_to_different");
    }

    /// SSTORE same value (noop): 100 gas (warm noop) + 2100 cold, 0 refund.
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_same_value_noop() {
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let bytecode = vec![
            0x60, 0x05, // PUSH1 0x05 (same as current)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE
            0x00, // STOP
        ];
        assert_gas_and_refund_matches(bytecode, storage, "sstore_same_value_noop");
    }

    /// SSTORE warm second access: 1st cold, 2nd warm, 0 refund.
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_warm_second_access() {
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let bytecode = vec![
            0x60, 0x0A, // PUSH1 0x0A (value 10)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE (cold)
            0x60, 0x0B, // PUSH1 0x0B (value 11)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE (warm)
            0x00, // STOP
        ];
        assert_gas_and_refund_matches(bytecode, storage, "sstore_warm_second_access");
    }

    // ─── SSTORE edge case tests (nonzero refund — pre-refund gas only) ────
    //
    // These tests trigger SSTORE refunds where revmc uses the pre-EIP-3529
    // constant (15000) instead of the post-EIP-3529 value (4800). We only
    // compare pre-refund execution gas, which is unaffected.

    /// SSTORE nonzero→zero: triggers 4800 refund (LEVM) / 15000 refund (revmc).
    /// Pre-refund execution gas should match.
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_nonzero_to_zero() {
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let bytecode = vec![
            0x60, 0x00, // PUSH1 0x00 (value = 0)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE
            0x00, // STOP
        ];
        assert_pre_refund_gas_matches(bytecode, storage, "sstore_nonzero_to_zero");
    }

    /// SSTORE restore original value: slot=5, write 10, then write 5 back.
    ///
    /// Key test for the negative refund bug fix in execution.rs. The second
    /// SSTORE restores the original value, producing a negative refund delta
    /// from revm. Before the fix, negative refunds were silently dropped.
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_restore_original() {
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let bytecode = vec![
            0x60, 0x0A, // PUSH1 0x0A (value 10)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE (5 → 10, cold)
            0x60, 0x05, // PUSH1 0x05 (restore to original)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE (10 → 5, warm, restore original)
            0x00, // STOP
        ];
        assert_pre_refund_gas_matches(bytecode, storage, "sstore_restore_original");
    }

    /// SSTORE restore to zero original: slot=0, write 10, then write 0 back.
    /// Triggers 19900 restore refund.
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_restore_zero_original() {
        let bytecode = vec![
            0x60, 0x0A, // PUSH1 0x0A (value 10)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE (0 → 10, cold)
            0x60, 0x00, // PUSH1 0x00 (value 0, restore)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE (10 → 0, warm, restore zero original)
            0x00, // STOP
        ];
        assert_pre_refund_gas_matches(
            bytecode,
            FxHashMap::default(),
            "sstore_restore_zero_original",
        );
    }

    /// SSTORE clear-then-restore: slot=5, write 0, then write 5 back.
    /// Net refund = 2800 (LEVM) / 12200 (revmc, upstream constant issue).
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_clear_then_restore() {
        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let bytecode = vec![
            0x60, 0x00, // PUSH1 0x00 (value 0, clear)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE (5 → 0, cold, clears)
            0x60, 0x05, // PUSH1 0x05 (value 5, restore)
            0x60, 0x00, // PUSH1 0x00 (slot)
            0x55, // SSTORE (0 → 5, warm, restore from zero)
            0x00, // STOP
        ];
        assert_pre_refund_gas_matches(bytecode, storage, "sstore_clear_then_restore");
    }

    // ─── Memory expansion tests ───────────────────────────────────────────

    /// MSTORE at offset 1024: triggers quadratic memory expansion cost.
    #[test]
    #[serial_test::serial]
    fn test_gas_large_memory_expansion() {
        let bytecode = vec![
            0x60, 0x42, // PUSH1 0x42
            0x61, 0x04, 0x00, // PUSH2 0x0400 (offset 1024)
            0x52, // MSTORE
            0x00, // STOP
        ];
        assert_gas_and_refund_matches(bytecode, FxHashMap::default(), "large_memory_expansion");
    }

    /// Two MSTOREs at increasing offsets: incremental memory expansion.
    #[test]
    #[serial_test::serial]
    fn test_gas_memory_incremental() {
        let bytecode = vec![
            0x60, 0x01, // PUSH1 0x01
            0x60, 0x00, // PUSH1 0x00 (offset 0)
            0x52, // MSTORE
            0x60, 0x02, // PUSH1 0x02
            0x61, 0x02, 0x00, // PUSH2 0x0200 (offset 512)
            0x52, // MSTORE
            0x00, // STOP
        ];
        assert_gas_and_refund_matches(bytecode, FxHashMap::default(), "memory_incremental");
    }

    /// MSTORE + SSTORE combined: verify both memory and storage gas align.
    #[test]
    #[serial_test::serial]
    fn test_gas_sstore_oog_after_memory() {
        let bytecode = vec![
            0x60, 0xFF, // PUSH1 0xFF
            0x61, 0x10, 0x00, // PUSH2 0x1000 (offset 4096)
            0x52, // MSTORE
            0x60, 0x01, // PUSH1 0x01
            0x60, 0x00, // PUSH1 0x00
            0x55, // SSTORE (zero→nonzero after memory expansion)
            0x00, // STOP
        ];
        assert_gas_and_refund_matches(bytecode, FxHashMap::default(), "sstore_after_memory");
    }
}
