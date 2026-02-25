//! EIP-7928 BAL (Block Access List) recording differential tests for JIT vs interpreter.
//!
//! Verifies that the JIT execution path produces identical BAL entries to the
//! interpreter path for SLOAD and SSTORE operations.
//!
//! Each test:
//! 1. Enables BAL recording on both interpreter and JIT databases
//! 2. Runs identical bytecode through both paths
//! 3. Compares the resulting `BlockAccessList` entries

#[cfg(test)]
#[cfg(feature = "revmc-backend")]
mod tests {
    use bytes::Bytes;
    use ethrex_common::types::Code;
    use ethrex_common::types::block_access_list::BlockAccessList;
    use ethrex_common::{Address, H256, U256};
    use ethrex_levm::call_frame::{CallFrame, Stack};
    use ethrex_levm::jit::cache::CodeCache;
    use ethrex_levm::memory::Memory;
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{JIT_STATE, Substate, VM, VMType};
    use rustc_hash::FxHashMap;

    use crate::backend::RevmcBackend;
    use crate::execution::execute_jit;
    use crate::tests::test_helpers::{
        TEST_GAS_LIMIT, make_contract_accounts, make_test_db, make_test_env, make_test_tx,
    };

    /// Run bytecode through the interpreter with BAL recording enabled.
    /// Returns the built BlockAccessList.
    ///
    /// Uses `execute()` instead of `stateless_execute()` because the latter
    /// calls `undo_last_transaction()` → `restore_cache_state()` which reverts
    /// BAL writes back to reads (correct for state rollback, but prevents
    /// comparing the actual BAL entries recorded during execution).
    fn run_interpreter_with_bal(code: Code, storage: FxHashMap<H256, U256>) -> BlockAccessList {
        let (contract_addr, sender_addr, accounts) = make_contract_accounts(code, storage);
        let mut db = make_test_db(accounts);

        // Enable BAL recording with block access index 1 (first tx)
        db.enable_bal_recording();
        db.set_bal_index(1);

        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM::new should succeed");

        let report = vm.execute().expect("execution should succeed");
        assert!(
            report.is_success(),
            "interpreter should succeed: {:?}",
            report.result
        );

        db.take_bal().expect("BAL should be present")
    }

    /// Run bytecode through the JIT with BAL recording enabled.
    /// Returns the built BlockAccessList.
    fn run_jit_with_bal(code: Code, storage: FxHashMap<H256, U256>) -> BlockAccessList {
        let fork = ethrex_common::types::Fork::Cancun;

        JIT_STATE.reset_for_testing();

        let backend = RevmcBackend::default();
        let code_cache = CodeCache::new();
        backend
            .compile_and_cache(&code, fork, &code_cache)
            .expect("JIT compilation should succeed");
        let compiled = code_cache
            .get(&(code.hash, fork))
            .expect("compiled code should be in cache");

        let (contract_addr, sender_addr, accounts) = make_contract_accounts(code.clone(), storage);
        let mut db = make_test_db(accounts);

        // Enable BAL recording with block access index 1 (first tx)
        db.enable_bal_recording();
        db.set_bal_index(1);

        let env = make_test_env(sender_addr);

        let mut call_frame = CallFrame::new(
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
            Stack::default(),
            Memory::default(),
        );

        let mut substate = Substate::default();
        let mut storage_original_values = FxHashMap::default();

        let outcome = execute_jit(
            &compiled,
            &mut call_frame,
            &mut db,
            &mut substate,
            &env,
            &mut storage_original_values,
        )
        .expect("JIT execution should succeed");

        assert!(
            matches!(outcome, ethrex_levm::jit::types::JitOutcome::Success { .. }),
            "JIT should succeed: {outcome:?}"
        );

        db.take_bal().expect("BAL should be present")
    }

    /// Compare two BAL results by checking that storage reads and changes match
    /// for the contract address.
    fn assert_bal_storage_matches(
        interp_bal: &BlockAccessList,
        jit_bal: &BlockAccessList,
        contract_addr: Address,
    ) {
        let interp_account = interp_bal
            .accounts()
            .iter()
            .find(|a| a.address == contract_addr);
        let jit_account = jit_bal
            .accounts()
            .iter()
            .find(|a| a.address == contract_addr);

        match (interp_account, jit_account) {
            (Some(interp), Some(jit)) => {
                // Compare storage reads (sorted sets of U256 slot keys)
                let mut interp_reads: Vec<U256> = interp.storage_reads.clone();
                let mut jit_reads: Vec<U256> = jit.storage_reads.clone();
                interp_reads.sort();
                jit_reads.sort();
                assert_eq!(
                    interp_reads, jit_reads,
                    "BAL storage_reads mismatch.\n  Interpreter: {interp_reads:?}\n  JIT: {jit_reads:?}"
                );

                // Compare storage changes (slot + post_value)
                let interp_changes: Vec<(U256, Vec<U256>)> = interp
                    .storage_changes
                    .iter()
                    .map(|sc| {
                        let values: Vec<U256> =
                            sc.slot_changes.iter().map(|c| c.post_value).collect();
                        (sc.slot, values)
                    })
                    .collect();
                let jit_changes: Vec<(U256, Vec<U256>)> = jit
                    .storage_changes
                    .iter()
                    .map(|sc| {
                        let values: Vec<U256> =
                            sc.slot_changes.iter().map(|c| c.post_value).collect();
                        (sc.slot, values)
                    })
                    .collect();
                assert_eq!(
                    interp_changes, jit_changes,
                    "BAL storage_changes mismatch.\n  Interpreter: {interp_changes:?}\n  JIT: {jit_changes:?}"
                );
            }
            (None, None) => {
                // Both have no entry for the contract — fine for pure-computation
            }
            _ => {
                panic!(
                    "BAL account presence mismatch for {contract_addr:?}.\n  Interpreter: {}\n  JIT: {}",
                    interp_account.is_some(),
                    jit_account.is_some()
                );
            }
        }
    }

    /// SLOAD + SSTORE counter contract: load slot 0, add 1, store back.
    /// BAL should record slot 0 as a storage change (read promoted to write).
    #[test]
    #[serial_test::serial]
    fn test_sload_sstore_bal_matches_interpreter() {
        use crate::tests::storage::make_counter_bytecode;

        let bytecode = Bytes::from(make_counter_bytecode());
        let code = Code::from_bytecode(bytecode);

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let interp_bal = run_interpreter_with_bal(code.clone(), storage.clone());
        let jit_bal = run_jit_with_bal(code, storage);

        let contract_addr = Address::from_low_u64_be(0x42);
        assert_bal_storage_matches(&interp_bal, &jit_bal, contract_addr);
    }

    /// Pure SLOAD bytecode (no SSTORE). BAL should have storage_reads only.
    ///
    /// ```text
    /// PUSH1 0x00  SLOAD   // load slot 0
    /// POP                  // discard
    /// PUSH1 0x01  SLOAD   // load slot 1
    /// POP
    /// STOP
    /// ```
    #[test]
    #[serial_test::serial]
    fn test_sload_only_bal_matches_interpreter() {
        let code = Code::from_bytecode(Bytes::from(vec![
            0x60, 0x00, // PUSH1 0x00
            0x54, // SLOAD
            0x50, // POP
            0x60, 0x01, // PUSH1 0x01
            0x54, // SLOAD
            0x50, // POP
            0x00, // STOP
        ]));

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(42u64));
        storage.insert(H256::from_low_u64_be(1), U256::from(99u64));

        let interp_bal = run_interpreter_with_bal(code.clone(), storage.clone());
        let jit_bal = run_jit_with_bal(code, storage);

        let contract_addr = Address::from_low_u64_be(0x42);
        assert_bal_storage_matches(&interp_bal, &jit_bal, contract_addr);

        // Verify both have reads and no changes
        let jit_account = jit_bal
            .accounts()
            .iter()
            .find(|a| a.address == contract_addr)
            .expect("contract should appear in BAL");
        assert!(
            !jit_account.storage_reads.is_empty(),
            "should have storage reads"
        );
        assert!(
            jit_account.storage_changes.is_empty(),
            "should have no storage changes (read-only)"
        );
    }

    /// SSTORE with same value (no-op). BAL should record as read, not write.
    ///
    /// ```text
    /// PUSH1 0x05  PUSH1 0x00  SSTORE   // store 5 to slot 0 (already 5)
    /// STOP
    /// ```
    #[test]
    #[serial_test::serial]
    fn test_sstore_noop_bal_matches_interpreter() {
        let code = Code::from_bytecode(Bytes::from(vec![
            0x60, 0x05, // PUSH1 0x05
            0x60, 0x00, // PUSH1 0x00
            0x55, // SSTORE (slot 0 = 5, same as current)
            0x00, // STOP
        ]));

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let interp_bal = run_interpreter_with_bal(code.clone(), storage.clone());
        let jit_bal = run_jit_with_bal(code, storage);

        let contract_addr = Address::from_low_u64_be(0x42);
        assert_bal_storage_matches(&interp_bal, &jit_bal, contract_addr);

        // Verify: no-op SSTORE should produce a read, not a write
        let jit_account = jit_bal
            .accounts()
            .iter()
            .find(|a| a.address == contract_addr)
            .expect("contract should appear in BAL");
        assert!(
            jit_account.storage_changes.is_empty(),
            "no-op SSTORE should not produce storage_changes"
        );
    }

    /// SSTORE with different value. BAL should record storage change.
    ///
    /// ```text
    /// PUSH1 0x0A  PUSH1 0x00  SSTORE   // store 10 to slot 0 (was 5)
    /// STOP
    /// ```
    #[test]
    #[serial_test::serial]
    fn test_sstore_change_bal_matches_interpreter() {
        let code = Code::from_bytecode(Bytes::from(vec![
            0x60, 0x0A, // PUSH1 10
            0x60, 0x00, // PUSH1 0x00
            0x55, // SSTORE (slot 0 = 10)
            0x00, // STOP
        ]));

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let interp_bal = run_interpreter_with_bal(code.clone(), storage.clone());
        let jit_bal = run_jit_with_bal(code, storage);

        let contract_addr = Address::from_low_u64_be(0x42);
        assert_bal_storage_matches(&interp_bal, &jit_bal, contract_addr);

        // Verify: actual write should produce a storage_change
        let jit_account = jit_bal
            .accounts()
            .iter()
            .find(|a| a.address == contract_addr)
            .expect("contract should appear in BAL");
        assert!(
            !jit_account.storage_changes.is_empty(),
            "SSTORE with different value should produce storage_changes"
        );
        // Post value should be 10
        let slot_change = &jit_account.storage_changes[0];
        assert_eq!(slot_change.slot, U256::zero());
        assert_eq!(slot_change.slot_changes[0].post_value, U256::from(10u64));
    }

    /// Multiple SSTOREs to the same slot. BAL should have the latest value.
    ///
    /// ```text
    /// PUSH1 0x0A  PUSH1 0x00  SSTORE   // slot 0 = 10
    /// PUSH1 0x14  PUSH1 0x00  SSTORE   // slot 0 = 20
    /// PUSH1 0x1E  PUSH1 0x00  SSTORE   // slot 0 = 30
    /// STOP
    /// ```
    #[test]
    #[serial_test::serial]
    fn test_multi_sstore_bal_matches_interpreter() {
        let code = Code::from_bytecode(Bytes::from(vec![
            0x60, 0x0A, // PUSH1 10
            0x60, 0x00, // PUSH1 0x00
            0x55, // SSTORE (slot 0 = 10)
            0x60, 0x14, // PUSH1 20
            0x60, 0x00, // PUSH1 0x00
            0x55, // SSTORE (slot 0 = 20)
            0x60, 0x1E, // PUSH1 30
            0x60, 0x00, // PUSH1 0x00
            0x55, // SSTORE (slot 0 = 30)
            0x00, // STOP
        ]));

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let interp_bal = run_interpreter_with_bal(code.clone(), storage.clone());
        let jit_bal = run_jit_with_bal(code, storage);

        let contract_addr = Address::from_low_u64_be(0x42);
        assert_bal_storage_matches(&interp_bal, &jit_bal, contract_addr);
    }
}
