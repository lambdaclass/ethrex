//! Integration tests for the dual-execution validation system (Phase 7 + G-3).
//!
//! Tests 1-3: Original Phase 7 dual-execution tests.
//! Tests G3-1..5: G-3 CALL/CREATE validation (guard removal).

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use std::sync::Arc;

    use ethrex_common::types::{Code, Fork, Transaction};
    use ethrex_common::{Address, H256, U256};
    use ethrex_levm::db::gen_db::GeneralizedDatabase;
    use ethrex_levm::jit::cache::CompiledCode;
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{VM, VMType};
    use rustc_hash::FxHashMap;

    use crate::tests::storage::make_counter_bytecode;
    use crate::tests::test_helpers::{
        CONTRACT_ADDR, SENDER_ADDR, make_contract_accounts, make_test_db, make_test_env,
        make_test_tx,
    };

    // ---------------------------------------------------------------
    // Shared helpers
    // ---------------------------------------------------------------

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

    /// Build bytecode that calls another contract via the given opcode, then returns.
    ///
    /// - `opcode = 0xF1` (CALL): pushes value arg (7 stack args)
    /// - `opcode = 0xFA` (STATICCALL) / `0xF4` (DELEGATECALL): no value arg (6 stack args)
    fn make_external_call_bytecode(target: Address, opcode: u8) -> Vec<u8> {
        let has_value = opcode == 0xF1; // CALL has a value argument
        let mut code = Vec::new();
        // retSize=0, retOffset=0, argsSize=0, argsOffset=0
        code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]);
        if has_value {
            code.extend_from_slice(&[0x60, 0x00]); // value = 0
        }
        code.push(0x73); // PUSH20 target
        let addr_bytes: [u8; 20] = target.into();
        code.extend_from_slice(&addr_bytes);
        code.extend_from_slice(&[0x62, 0xFF, 0xFF, 0xFF]); // PUSH3 gas
        code.push(opcode);
        // Store result and return 32 bytes
        code.extend_from_slice(&[0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
        code
    }

    /// Helper: create a VM setup for a caller contract that does an external call.
    ///
    /// Returns `(db, env, tx, caller_code)`. The caller contract lives at CONTRACT_ADDR
    /// and the target(s) are pre-seeded with STOP bytecode.
    fn setup_call_vm(
        caller_bytecode: Vec<u8>,
        targets: &[(Address, Vec<u8>)],
    ) -> (
        GeneralizedDatabase,
        ethrex_levm::Environment,
        Transaction,
        Code,
    ) {
        let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
        let sender_addr = Address::from_low_u64_be(SENDER_ADDR);
        let caller_code = Code::from_bytecode(Bytes::from(caller_bytecode));

        let mut cache = FxHashMap::default();
        cache.insert(
            contract_addr,
            ethrex_common::types::Account::new(
                U256::MAX,
                caller_code.clone(),
                0,
                FxHashMap::default(),
            ),
        );
        for (addr, bytecode) in targets {
            cache.insert(
                *addr,
                ethrex_common::types::Account::new(
                    U256::zero(),
                    Code::from_bytecode(Bytes::from(bytecode.clone())),
                    0,
                    FxHashMap::default(),
                ),
            );
        }
        cache.insert(
            sender_addr,
            ethrex_common::types::Account::new(
                U256::MAX,
                Code::from_bytecode(Bytes::new()),
                0,
                FxHashMap::default(),
            ),
        );
        cache.insert(
            Address::zero(),
            ethrex_common::types::Account::new(
                U256::zero(),
                Code::from_bytecode(Bytes::new()),
                0,
                FxHashMap::default(),
            ),
        );

        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = ethrex_common::types::BlockHeader {
            state_root: *ethrex_common::constants::EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );
        let db = GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        (db, env, tx, caller_code)
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
        let (jit_execs, _, _, _, validation_successes, validation_mismatches, _, _) =
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

    /// Mock backend that returns deliberately wrong gas (gas_used=1) to trigger mismatch.
    /// Reused across test 2 and all G-3 tests.
    struct MismatchBackend;

    impl MismatchBackend {
        fn register() {
            use ethrex_levm::vm::JIT_STATE;
            JIT_STATE.register_backend(Arc::new(Self));
        }
    }

    impl ethrex_levm::jit::dispatch::JitBackend for MismatchBackend {
        fn execute(
            &self,
            _compiled: &CompiledCode,
            _call_frame: &mut ethrex_levm::call_frame::CallFrame,
            _db: &mut GeneralizedDatabase,
            _substate: &mut ethrex_levm::vm::Substate,
            _env: &ethrex_levm::environment::Environment,
            _storage_original_values: &mut ethrex_levm::jit::dispatch::StorageOriginalValues,
        ) -> Result<ethrex_levm::jit::types::JitOutcome, String> {
            Ok(ethrex_levm::jit::types::JitOutcome::Success {
                gas_used: 1,
                output: Bytes::from(vec![0u8; 32]),
            })
        }

        fn execute_resume(
            &self,
            _resume_state: ethrex_levm::jit::types::JitResumeState,
            _sub_result: ethrex_levm::jit::types::SubCallResult,
            _call_frame: &mut ethrex_levm::call_frame::CallFrame,
            _db: &mut GeneralizedDatabase,
            _substate: &mut ethrex_levm::vm::Substate,
            _env: &ethrex_levm::environment::Environment,
            _storage_original_values: &mut ethrex_levm::jit::dispatch::StorageOriginalValues,
        ) -> Result<ethrex_levm::jit::types::JitOutcome, String> {
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

    /// Integration test: mismatch triggers cache invalidation.
    ///
    /// Registers a mock backend that returns deliberately wrong gas_used,
    /// inserts a dummy `CompiledCode` into `JIT_STATE.cache`, and runs
    /// `stateless_execute()`. The validation detects the gas mismatch,
    /// invalidates the cache entry, and increments `validation_mismatches`.
    #[test]
    #[serial_test::serial]
    fn test_dual_execution_mismatch_invalidates_cache() {
        use ethrex_levm::vm::JIT_STATE;

        let fork = Fork::Cancun;

        // Reset JIT state for test isolation
        JIT_STATE.reset_for_testing();
        MismatchBackend::register();

        let (mut db, env, tx, counter_code) = setup_counter_vm();

        // Insert dummy compiled code into cache (null pointer — mock doesn't dereference it)
        let cache_key = (counter_code.hash, fork);
        #[expect(unsafe_code)]
        let dummy_compiled = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, false) };
        JIT_STATE.cache.insert(cache_key, dummy_compiled);
        assert!(JIT_STATE.cache.get(&cache_key).is_some());

        // Capture baseline metrics
        let (_, _, _, _, baseline_successes, baseline_mismatches, _, _) =
            JIT_STATE.metrics.snapshot();

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
        let (_, _, _, _, final_successes, final_mismatches, _, _) = JIT_STATE.metrics.snapshot();
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

        use ethrex_common::types::{Account, AccountState, ChainConfig, Code, CodeMetadata};

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
        let (_, _, _, _, baseline_successes, baseline_mismatches, _, _) =
            JIT_STATE.metrics.snapshot();

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
        let (_, _, _, _, final_successes, final_mismatches, _, _) = JIT_STATE.metrics.snapshot();
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

    // ---------------------------------------------------------------
    // G-3: CALL/CREATE Dual-Execution Validation
    //
    // These tests verify that bytecodes with has_external_calls=true
    // are still validated via dual-execution (the guard was removed).
    // All G-3 tests reuse `MismatchBackend` and `setup_call_vm()`.
    // ---------------------------------------------------------------

    /// Helper: run a mismatch-backend test for a given opcode and assert validation ran.
    ///
    /// Builds bytecode with `make_external_call_bytecode(target, opcode)`,
    /// inserts a dummy `CompiledCode` with `has_external_calls=true`,
    /// runs `stateless_execute()`, and returns `(new_successes, new_mismatches)`.
    fn run_g3_mismatch_test(opcode: u8) -> (u64, u64) {
        use ethrex_levm::vm::JIT_STATE;

        let fork = Fork::Cancun;
        let target_addr = Address::from_low_u64_be(0xBEEF);

        JIT_STATE.reset_for_testing();
        MismatchBackend::register();

        let bytecode = make_external_call_bytecode(target_addr, opcode);
        let (mut db, env, tx, caller_code) = setup_call_vm(bytecode, &[(target_addr, vec![0x00])]);

        let cache_key = (caller_code.hash, fork);
        #[expect(unsafe_code)]
        let dummy = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, true) };
        JIT_STATE.cache.insert(cache_key, dummy);

        let (_, _, _, _, base_s, base_m, _, _) = JIT_STATE.metrics.snapshot();

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");
        let _report = vm.stateless_execute().expect("execution should succeed");

        let (_, _, _, _, final_s, final_m, _, _) = JIT_STATE.metrics.snapshot();
        (
            final_s.saturating_sub(base_s),
            final_m.saturating_sub(base_m),
        )
    }

    /// G-3 Test 1: Validation runs for CALL bytecodes (has_external_calls=true).
    #[test]
    #[serial_test::serial]
    fn test_g3_validation_runs_for_call_bytecode() {
        let (successes, mismatches) = run_g3_mismatch_test(0xF1); // CALL
        assert!(
            successes + mismatches > 0,
            "G-3: validation must run for CALL bytecodes (s={successes}, m={mismatches})"
        );
    }

    /// G-3 Test 2: STATICCALL mismatch invalidates cache.
    #[test]
    #[serial_test::serial]
    fn test_g3_staticcall_mismatch_invalidates_cache() {
        use ethrex_levm::vm::JIT_STATE;

        let fork = Fork::Cancun;
        let target_addr = Address::from_low_u64_be(0xCAFE);

        JIT_STATE.reset_for_testing();
        MismatchBackend::register();

        let bytecode = make_external_call_bytecode(target_addr, 0xFA); // STATICCALL
        let (mut db, env, tx, caller_code) = setup_call_vm(bytecode, &[(target_addr, vec![0x00])]);

        let cache_key = (caller_code.hash, fork);
        #[expect(unsafe_code)]
        let dummy = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, true) };
        JIT_STATE.cache.insert(cache_key, dummy);

        let (_, _, _, _, _, base_m, _, _) = JIT_STATE.metrics.snapshot();

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");
        let _report = vm.stateless_execute().expect("execution should succeed");

        let (_, _, _, _, _, final_m, _, _) = JIT_STATE.metrics.snapshot();
        assert!(
            final_m.saturating_sub(base_m) > 0,
            "G-3: STATICCALL must trigger mismatch"
        );
        assert!(
            JIT_STATE.cache.get(&cache_key).is_none(),
            "cache should be invalidated after STATICCALL mismatch"
        );
    }

    /// G-3 Test 3: Validation runs for DELEGATECALL bytecode.
    #[test]
    #[serial_test::serial]
    fn test_g3_delegatecall_validation_runs() {
        let (successes, mismatches) = run_g3_mismatch_test(0xF4); // DELEGATECALL
        assert!(
            successes + mismatches > 0,
            "G-3: validation must run for DELEGATECALL bytecodes (s={successes}, m={mismatches})"
        );
    }

    /// G-3 Test 4: Pure-computation validation still works (regression).
    #[test]
    #[serial_test::serial]
    fn test_g3_regression_pure_computation_still_validates() {
        use ethrex_levm::vm::JIT_STATE;

        let fork = Fork::Cancun;

        JIT_STATE.reset_for_testing();
        MismatchBackend::register();

        let (mut db, env, tx, counter_code) = setup_counter_vm();

        let cache_key = (counter_code.hash, fork);
        #[expect(unsafe_code)]
        let dummy = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, false) };
        JIT_STATE.cache.insert(cache_key, dummy);

        let (_, _, _, _, _, base_m, _, _) = JIT_STATE.metrics.snapshot();

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");
        let report = vm
            .stateless_execute()
            .expect("execution should succeed (interpreter fallback)");

        assert!(report.is_success(), "should succeed via interpreter");
        assert_eq!(
            U256::from_big_endian(&report.output),
            U256::from(6u64),
            "pure computation should produce 6"
        );

        let (_, _, _, _, _, final_m, _, _) = JIT_STATE.metrics.snapshot();
        assert!(
            final_m.saturating_sub(base_m) > 0,
            "pure computation validation should still detect mismatch after G-3 changes"
        );
    }

    /// G-3 Test 5: Both pure and CALL bytecodes are validated (total >= 2).
    #[test]
    #[serial_test::serial]
    fn test_g3_both_pure_and_call_bytecodes_validated() {
        use ethrex_levm::vm::JIT_STATE;

        let fork = Fork::Cancun;

        JIT_STATE.reset_for_testing();
        MismatchBackend::register();

        // --- Run 1: pure computation (has_external_calls=false) ---
        {
            let (mut db, env, tx, counter_code) = setup_counter_vm();
            let cache_key = (counter_code.hash, fork);
            #[expect(unsafe_code)]
            let dummy = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, false) };
            JIT_STATE.cache.insert(cache_key, dummy);

            let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
                .expect("VM::new");
            let _ = vm.stateless_execute().expect("exec 1");
        }

        // --- Run 2: CALL bytecode (has_external_calls=true) ---
        {
            let target_addr = Address::from_low_u64_be(0xBEEF);
            let bytecode = make_external_call_bytecode(target_addr, 0xF1);
            let (mut db, env, tx, caller_code) =
                setup_call_vm(bytecode, &[(target_addr, vec![0x00])]);

            let cache_key = (caller_code.hash, fork);
            #[expect(unsafe_code)]
            let dummy = unsafe { CompiledCode::new(std::ptr::null(), 100, 5, None, true) };
            JIT_STATE.cache.insert(cache_key, dummy);

            let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
                .expect("VM::new");
            let _ = vm.stateless_execute().expect("exec 2");
        }

        let (_, _, _, _, final_s, final_m, _, _) = JIT_STATE.metrics.snapshot();
        let total = final_s + final_m;
        assert!(
            total >= 2,
            "G-3: both bytecodes must be validated, total={total} (s={final_s}, m={final_m})"
        );
    }
}
