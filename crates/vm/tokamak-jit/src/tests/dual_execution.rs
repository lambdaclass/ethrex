//! Integration tests for the dual-execution validation system (Phase 7).
//!
//! Test 1: Real JIT compilation (revmc) of a pure-computation counter contract,
//! exercised through the full VM dispatch path. Verifies that JIT and interpreter
//! produce identical results and that `validation_successes` metric increments.
//!
//! Test 2: Mock backend that returns deliberately wrong gas, exercised through
//! the full VM dispatch path. Verifies that mismatch triggers cache invalidation
//! and `validation_mismatches` metric increments.

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use std::sync::Arc;

    use ethrex_common::types::{
        Account, BlockHeader, Code, EIP1559Transaction, Fork, Transaction, TxKind,
    };
    use ethrex_common::{constants::EMPTY_TRIE_HASH, Address, H256, U256};
    use ethrex_levm::db::gen_db::GeneralizedDatabase;
    use ethrex_levm::jit::cache::CompiledCode;
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{VMType, VM};
    use rustc_hash::FxHashMap;

    use crate::tests::storage::make_counter_bytecode;

    /// Helper: create the standard counter contract VM setup.
    ///
    /// Returns `(db, env, tx, counter_code)` ready for `VM::new()`.
    /// Pre-seeds storage slot 0 = 5, so counter returns 6.
    fn setup_counter_vm() -> (
        GeneralizedDatabase,
        ethrex_levm::Environment,
        Transaction,
        Code,
    ) {
        let contract_addr = Address::from_low_u64_be(0x42);
        let sender_addr = Address::from_low_u64_be(0x100);

        let bytecode = Bytes::from(make_counter_bytecode());
        let counter_code = Code::from_bytecode(bytecode);

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

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
        let db = GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache);

        #[expect(clippy::as_conversions)]
        let gas = (i64::MAX - 1) as u64;
        let env = ethrex_levm::Environment {
            origin: sender_addr,
            gas_limit: gas,
            block_gas_limit: gas,
            ..Default::default()
        };
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(contract_addr),
            data: Bytes::new(),
            ..Default::default()
        });

        (db, env, tx, counter_code)
    }

    /// Integration test: dual execution produces Match for a pure-computation contract.
    ///
    /// Compiles the counter contract via revmc/LLVM, inserts into `JIT_STATE.cache`,
    /// registers the real backend, and runs through `stateless_execute()`.
    /// The full validation path (snapshot → JIT → swap → interpreter → compare) runs,
    /// and we verify `validation_successes` increments.
    #[cfg(feature = "revmc-backend")]
    #[test]
    #[serial_test::serial]
    fn test_dual_execution_match_via_full_vm() {
        use ethrex_levm::vm::JIT_STATE;

        use crate::backend::RevmcBackend;

        let fork = Fork::Cancun;

        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        // Register backend
        let backend = Arc::new(RevmcBackend::default());
        JIT_STATE.register_backend(backend.clone());

        let (mut db, env, tx, counter_code) = setup_counter_vm();

        // Pre-compile and insert into JIT_STATE.cache
        backend
            .compile_and_cache(&counter_code, fork, &JIT_STATE.cache)
            .expect("compilation should succeed");
        assert!(
            JIT_STATE
                .cache
                .get(&(counter_code.hash, fork))
                .is_some(),
            "compiled code should be in JIT_STATE cache"
        );

        // Run VM (JIT will dispatch since code is in cache, validation runs since
        // validation_mode=true and validation_counts=0 < max_validation_runs=3)
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("counter execution should succeed");

        // Verify execution correctness
        assert!(
            report.is_success(),
            "counter should succeed, got: {:?}",
            report.result
        );
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(result_val, U256::from(6u64), "5 + 1 = 6");

        // Verify dual execution validation happened and matched
        let (jit_execs, _, _, _, validation_successes, validation_mismatches) =
            JIT_STATE.metrics.snapshot();
        assert_eq!(
            validation_successes, 1,
            "should have 1 successful validation"
        );
        assert_eq!(
            validation_mismatches, 0,
            "should have no validation mismatches"
        );
        assert!(jit_execs >= 1, "should have at least 1 JIT execution");

        // Verify cache entry is still present (not invalidated)
        assert!(
            JIT_STATE
                .cache
                .get(&(counter_code.hash, fork))
                .is_some(),
            "cache entry should still exist after successful validation"
        );
    }

    /// Integration test: mismatch triggers cache invalidation.
    ///
    /// Registers a mock backend that returns deliberately wrong gas_used,
    /// inserts a dummy `CompiledCode` into `JIT_STATE.cache`, and runs
    /// `stateless_execute()`. The validation detects the gas mismatch,
    /// invalidates the cache entry, and increments `validation_mismatches`.
    #[test]
    #[serial_test::serial]
    fn test_dual_execution_mismatch_invalidates_cache() {
        use ethrex_levm::call_frame::CallFrame;
        use ethrex_levm::environment::Environment;
        use ethrex_levm::jit::dispatch::{JitBackend, StorageOriginalValues};
        use ethrex_levm::jit::types::{JitOutcome, JitResumeState, SubCallResult};
        use ethrex_levm::vm::{Substate, JIT_STATE};

        /// Mock backend that returns deliberately wrong gas to trigger mismatch.
        struct MismatchBackend;

        impl JitBackend for MismatchBackend {
            fn execute(
                &self,
                _compiled: &CompiledCode,
                _call_frame: &mut CallFrame,
                _db: &mut GeneralizedDatabase,
                _substate: &mut Substate,
                _env: &Environment,
                _storage_original_values: &mut StorageOriginalValues,
            ) -> Result<JitOutcome, String> {
                // Return deliberately wrong gas_used to trigger mismatch
                Ok(JitOutcome::Success {
                    gas_used: 1,
                    output: Bytes::from(vec![0u8; 32]),
                })
            }

            fn execute_resume(
                &self,
                _resume_state: JitResumeState,
                _sub_result: SubCallResult,
                _call_frame: &mut CallFrame,
                _db: &mut GeneralizedDatabase,
                _substate: &mut Substate,
                _env: &Environment,
                _storage_original_values: &mut StorageOriginalValues,
            ) -> Result<JitOutcome, String> {
                Err("not implemented".to_string())
            }

            fn compile(
                &self,
                _code: &ethrex_common::types::Code,
                _fork: Fork,
                _cache: &ethrex_levm::jit::cache::CodeCache,
            ) -> Result<(), String> {
                Ok(())
            }
        }

        let fork = Fork::Cancun;

        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();

        // Register mock backend that produces wrong results
        JIT_STATE.register_backend(Arc::new(MismatchBackend));

        let (mut db, env, tx, counter_code) = setup_counter_vm();

        // Insert dummy compiled code into cache (null pointer — mock doesn't dereference it)
        let cache_key = (counter_code.hash, fork);
        #[expect(unsafe_code)]
        let dummy_compiled =
            unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, false) };
        JIT_STATE.cache.insert(cache_key, dummy_compiled);
        assert!(JIT_STATE.cache.get(&cache_key).is_some());

        // Capture baseline metrics (non-serial tests may run concurrently and
        // modify JIT_STATE, so we compare deltas instead of absolute values).
        let (_, _, _, _, baseline_successes, baseline_mismatches) =
            JIT_STATE.metrics.snapshot();

        // Run VM — JIT dispatches to mock backend, validation detects mismatch
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
        )
        .expect("VM::new should succeed");

        let report = vm
            .stateless_execute()
            .expect("execution should succeed (interpreter fallback)");

        // The VM should still return a valid result (from interpreter fallback)
        assert!(
            report.is_success(),
            "counter should succeed via interpreter, got: {:?}",
            report.result
        );
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(6u64),
            "interpreter should produce correct result"
        );

        // Verify mismatch was detected (compare delta from baseline)
        let (_, _, _, _, final_successes, final_mismatches) =
            JIT_STATE.metrics.snapshot();
        assert_eq!(
            final_mismatches.saturating_sub(baseline_mismatches),
            1,
            "should have exactly 1 new validation mismatch (baseline={baseline_mismatches}, final={final_mismatches})"
        );
        assert_eq!(
            final_successes.saturating_sub(baseline_successes),
            0,
            "should have no new successful validations"
        );

        // Verify cache entry was invalidated
        assert!(
            JIT_STATE.cache.get(&cache_key).is_none(),
            "cache entry should be invalidated after mismatch"
        );
    }
}
