//! Integration tests for the dual-execution validation system (Phase 7).
//!
//! Test 1: Real JIT compilation (revmc) of a pure-computation counter contract,
//! exercised through the full VM dispatch path. Verifies that JIT and interpreter
//! produce identical results and that `validation_successes` metric increments.
//!
//! Test 2: Mock backend that returns deliberately wrong gas, exercised through
//! the full VM dispatch path. Verifies that mismatch triggers cache invalidation
//! and `validation_mismatches` metric increments.
//!
//! Test 3: Mock backend that succeeds, but interpreter replay fails with
//! InternalError (FailingDatabase). Verifies the swap-back recovery path:
//! VM restores JIT state and returns the JIT result, with no validation
//! counters incremented.

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use std::sync::Arc;

    use ethrex_common::types::{
        Code, Fork, Transaction,
    };
    use ethrex_common::{Address, H256, U256};
    use ethrex_levm::db::gen_db::GeneralizedDatabase;
    use ethrex_levm::jit::cache::CompiledCode;
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{VM, VMType};
    use rustc_hash::FxHashMap;

    use crate::tests::storage::make_counter_bytecode;
    use crate::tests::test_helpers::{
        make_contract_accounts, make_test_db, make_test_env, make_test_tx,
    };

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
        let bytecode = Bytes::from(make_counter_bytecode());
        let counter_code = Code::from_bytecode(bytecode);

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let (contract_addr, sender_addr, accounts) =
            make_contract_accounts(counter_code.clone(), storage);
        let db = make_test_db(accounts);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

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
            JIT_STATE.cache.get(&(counter_code.hash, fork)).is_some(),
            "compiled code should be in JIT_STATE cache"
        );

        // Run VM (JIT will dispatch since code is in cache, validation runs since
        // validation_mode=true and validation_counts=0 < max_validation_runs=3)
        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
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
            JIT_STATE.cache.get(&(counter_code.hash, fork)).is_some(),
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
        use ethrex_levm::vm::{JIT_STATE, Substate};

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
        let dummy_compiled = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, false) };
        JIT_STATE.cache.insert(cache_key, dummy_compiled);
        assert!(JIT_STATE.cache.get(&cache_key).is_some());

        // Capture baseline metrics (non-serial tests may run concurrently and
        // modify JIT_STATE, so we compare deltas instead of absolute values).
        let (_, _, _, _, baseline_successes, baseline_mismatches) = JIT_STATE.metrics.snapshot();

        // Run VM — JIT dispatches to mock backend, validation detects mismatch
        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
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
        let (_, _, _, _, final_successes, final_mismatches) = JIT_STATE.metrics.snapshot();
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

    /// Integration test: interpreter replay failure triggers swap-back recovery.
    ///
    /// Registers a mock backend that returns a successful JIT result without
    /// touching the database. The backing store is a `FailingDatabase` that
    /// errors on all reads. The bytecode includes BALANCE on an uncached
    /// address, causing `interpreter_loop` to fail with `InternalError`.
    ///
    /// Verifies:
    /// - VM returns successfully (JIT result, not interpreter error)
    /// - No `validation_successes` or `validation_mismatches` incremented
    ///   (validation was inconclusive)
    /// - Cache entry remains (not invalidated — mismatch was not proven)
    #[test]
    #[serial_test::serial]
    fn test_interpreter_err_swaps_back_to_jit_state() {
        use ethrex_levm::call_frame::CallFrame;
        use ethrex_levm::db::Database;
        use ethrex_levm::environment::Environment;
        use ethrex_levm::errors::DatabaseError;
        use ethrex_levm::jit::dispatch::{JitBackend, StorageOriginalValues};
        use ethrex_levm::jit::types::{JitOutcome, JitResumeState, SubCallResult};
        use ethrex_levm::vm::{JIT_STATE, Substate};

        use ethrex_common::types::{
            Account, AccountState, ChainConfig, Code, CodeMetadata,
        };

        use crate::tests::test_helpers::{CONTRACT_ADDR, SENDER_ADDR};

        /// Database that always returns errors.
        /// Forces `interpreter_loop` to fail with InternalError when it
        /// tries to load an uncached account.
        struct FailingDatabase;

        impl Database for FailingDatabase {
            fn get_account_state(&self, _: Address) -> Result<AccountState, DatabaseError> {
                Err(DatabaseError::Custom(
                    "deliberately failing store".to_string(),
                ))
            }
            fn get_storage_value(&self, _: Address, _: H256) -> Result<U256, DatabaseError> {
                Err(DatabaseError::Custom(
                    "deliberately failing store".to_string(),
                ))
            }
            fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
                Err(DatabaseError::Custom(
                    "deliberately failing store".to_string(),
                ))
            }
            fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
                Err(DatabaseError::Custom(
                    "deliberately failing store".to_string(),
                ))
            }
            fn get_account_code(&self, _: H256) -> Result<Code, DatabaseError> {
                Err(DatabaseError::Custom(
                    "deliberately failing store".to_string(),
                ))
            }
            fn get_code_metadata(&self, _: H256) -> Result<CodeMetadata, DatabaseError> {
                Err(DatabaseError::Custom(
                    "deliberately failing store".to_string(),
                ))
            }
        }

        /// Mock backend that returns successful JIT result without touching DB.
        struct SuccessBackend;

        impl JitBackend for SuccessBackend {
            fn execute(
                &self,
                _compiled: &CompiledCode,
                _call_frame: &mut CallFrame,
                _db: &mut GeneralizedDatabase,
                _substate: &mut Substate,
                _env: &Environment,
                _storage_original_values: &mut StorageOriginalValues,
            ) -> Result<JitOutcome, String> {
                let mut output = vec![0u8; 32];
                output[31] = 0x42;
                Ok(JitOutcome::Success {
                    gas_used: 50000,
                    output: Bytes::from(output),
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
                _code: &Code,
                _fork: Fork,
                _cache: &ethrex_levm::jit::cache::CodeCache,
            ) -> Result<(), String> {
                Ok(())
            }
        }

        // Bytecode: PUSH20 0xDEAD, BALANCE, POP, PUSH1 0x42, PUSH1 0, MSTORE, PUSH1 32, PUSH1 0, RETURN
        // The BALANCE of uncached address 0xDEAD forces a DB read → FailingDatabase → InternalError
        let mut bytecode_bytes = Vec::new();
        // PUSH20 <0xDEAD padded to 20 bytes>
        bytecode_bytes.push(0x73);
        bytecode_bytes.extend_from_slice(&[0u8; 18]);
        bytecode_bytes.push(0xDE);
        bytecode_bytes.push(0xAD);
        // BALANCE
        bytecode_bytes.push(0x31);
        // POP
        bytecode_bytes.push(0x50);
        // PUSH1 0x42, PUSH1 0x00, MSTORE (store 0x42 in memory)
        bytecode_bytes.extend_from_slice(&[0x60, 0x42, 0x60, 0x00, 0x52]);
        // PUSH1 0x20, PUSH1 0x00, RETURN (return 32 bytes from memory offset 0)
        bytecode_bytes.extend_from_slice(&[0x60, 0x20, 0x60, 0x00, 0xf3]);

        let fork = Fork::Cancun;

        let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
        let sender_addr = Address::from_low_u64_be(SENDER_ADDR);

        let code = Code::from_bytecode(Bytes::from(bytecode_bytes));

        // Build account cache — pre-cache contract, sender, and coinbase (Address::zero)
        // so VM::new and finalize_execution don't hit the FailingDatabase.
        let mut cache = FxHashMap::default();
        cache.insert(
            contract_addr,
            Account::new(U256::MAX, code.clone(), 0, FxHashMap::default()),
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
        // Pre-cache coinbase (default Address::zero) to avoid DB read in finalize
        cache.insert(
            Address::zero(),
            Account::new(
                U256::zero(),
                Code::from_bytecode(Bytes::new()),
                0,
                FxHashMap::default(),
            ),
        );

        let store: Arc<dyn Database> = Arc::new(FailingDatabase);
        let mut db = GeneralizedDatabase::new_with_account_state(store, cache);

        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        // Reset JIT state and register mock backend
        JIT_STATE.reset_for_testing();
        JIT_STATE.register_backend(Arc::new(SuccessBackend));

        // Insert dummy compiled code (has_external_calls = false so validation triggers)
        let cache_key = (code.hash, fork);
        #[expect(unsafe_code)]
        let dummy_compiled = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, false) };
        JIT_STATE.cache.insert(cache_key, dummy_compiled);

        // Capture baseline metrics
        let (_, _, _, _, baseline_successes, baseline_mismatches) = JIT_STATE.metrics.snapshot();

        // Run VM — JIT succeeds, interpreter fails on BALANCE(0xDEAD), swap-back fires
        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed (all needed accounts pre-cached)");

        let report = vm
            .stateless_execute()
            .expect("execution should succeed (JIT result via swap-back)");

        // Verify execution succeeded with JIT result
        assert!(
            report.is_success(),
            "should succeed via JIT swap-back, got: {:?}",
            report.result
        );
        let result_val = U256::from_big_endian(&report.output);
        assert_eq!(
            result_val,
            U256::from(0x42u64),
            "output should match JIT mock (0x42)"
        );

        // Verify no validation counters changed (inconclusive, not match/mismatch)
        let (_, _, _, _, final_successes, final_mismatches) = JIT_STATE.metrics.snapshot();
        assert_eq!(
            final_successes.saturating_sub(baseline_successes),
            0,
            "should have no new validation successes (inconclusive)"
        );
        assert_eq!(
            final_mismatches.saturating_sub(baseline_mismatches),
            0,
            "should have no new validation mismatches (inconclusive)"
        );

        // Verify cache entry is still present (not invalidated — no proven mismatch)
        assert!(
            JIT_STATE.cache.get(&cache_key).is_some(),
            "cache entry should remain after inconclusive validation"
        );
    }
}
