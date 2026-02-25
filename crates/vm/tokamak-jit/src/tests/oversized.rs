//! Oversized bytecode tests for the JIT compiler.
//!
//! Validates the graceful interpreter fallback when bytecode exceeds
//! `max_bytecode_size` (EIP-170: 24576 bytes). Tests cover:
//! - VM dispatch correctly skips JIT and falls back to interpreter
//! - Boundary condition: exactly max size CAN compile
//! - Backend rejects oversized with `BytecodeTooLarge`

#[cfg(test)]
#[cfg(feature = "revmc-backend")]
mod tests {
    use std::sync::atomic::Ordering;

    use bytes::Bytes;
    use ethrex_common::types::{Code, Fork};
    use ethrex_levm::jit::cache::CodeCache;
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{JIT_STATE, VM, VMType};
    use rustc_hash::FxHashMap;

    use crate::backend::RevmcBackend;
    use crate::error::JitError;
    use crate::tests::test_helpers::{
        make_contract_accounts, make_test_db, make_test_env, make_test_tx,
    };

    /// Build bytecode of a specific size that executes successfully.
    ///
    /// Fills with JUMPDEST (0x5b) as padding and ends with STOP (0x00).
    fn make_bytecode_of_size(size: usize) -> Vec<u8> {
        assert!(size >= 1, "need at least 1 byte for STOP");
        let mut code = vec![0x5b; size]; // JUMPDEST padding
        code[size - 1] = 0x00; // STOP at the end
        code
    }

    #[test]
    fn test_oversized_bytecode_falls_back_to_interpreter() {
        JIT_STATE.reset_for_testing();
        tokamak_jit::register_jit_backend();

        let max_size = JIT_STATE.config.max_bytecode_size;
        let oversized = make_bytecode_of_size(max_size + 1);
        let code = Code::from_bytecode(Bytes::from(oversized));
        let bytecode_hash = code.hash;

        let (contract_addr, sender_addr, accounts) =
            make_contract_accounts(code, FxHashMap::default());
        let mut db = make_test_db(accounts);
        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());

        // Run past the compilation threshold so the size gate fires
        let threshold = JIT_STATE.config.compilation_threshold;
        for _ in 0..=threshold {
            let mut db_clone = db.clone();
            let mut vm = VM::new(
                VMType::Transaction,
                &env,
                &tx,
                &mut db_clone,
                LevmCallTracer::new_non_active(),
            )
            .expect("VM creation");

            let result = vm.execute();
            assert!(result.is_ok(), "interpreter fallback should succeed");
        }

        // Verify: bytecode was marked oversized
        assert!(
            JIT_STATE.is_oversized(&bytecode_hash),
            "bytecode should be marked as oversized"
        );

        // Verify: compilation_skips was incremented
        assert!(
            JIT_STATE.metrics.compilation_skips.load(Ordering::Relaxed) > 0,
            "compilation_skips should be > 0"
        );

        // Verify: cache is empty (no JIT entry for this bytecode)
        assert!(
            JIT_STATE
                .cache
                .get(&(bytecode_hash, Fork::Cancun))
                .is_none(),
            "oversized bytecode should not be in the JIT cache"
        );

        // Additional runs should short-circuit via is_oversized (no repeated work)
        let skips_before = JIT_STATE.metrics.compilation_skips.load(Ordering::Relaxed);
        for _ in 0..5 {
            let mut db_clone = db.clone();
            let mut vm = VM::new(
                VMType::Transaction,
                &env,
                &tx,
                &mut db_clone,
                LevmCallTracer::new_non_active(),
            )
            .expect("VM creation");
            let result = vm.execute();
            assert!(result.is_ok(), "subsequent runs should still succeed");
        }
        let skips_after = JIT_STATE.metrics.compilation_skips.load(Ordering::Relaxed);
        // No additional skips — the is_oversized check prevents reaching the threshold check
        assert_eq!(
            skips_before, skips_after,
            "no additional compilation_skips after initial marking"
        );
    }

    #[test]
    fn test_exactly_max_size_compiles() {
        JIT_STATE.reset_for_testing();
        tokamak_jit::register_jit_backend();

        let max_size = JIT_STATE.config.max_bytecode_size;
        let exactly_max = make_bytecode_of_size(max_size);
        let code = Code::from_bytecode(Bytes::from(exactly_max));

        let backend = RevmcBackend::default();
        let cache = CodeCache::with_max_entries(64);

        // Should compile without error — boundary is inclusive
        let result = backend.compile_and_cache(&code, Fork::Cancun, &cache);
        assert!(
            result.is_ok(),
            "bytecode of exactly max_bytecode_size should compile: {:?}",
            result.err()
        );

        // Verify cache entry exists
        assert!(
            cache.get(&(code.hash, Fork::Cancun)).is_some(),
            "compiled code should be in cache"
        );

        // Hash should NOT be in oversized set
        assert!(
            !JIT_STATE.is_oversized(&code.hash),
            "exactly-max bytecode should not be marked oversized"
        );
    }

    #[test]
    fn test_backend_rejects_oversized() {
        JIT_STATE.reset_for_testing();

        let max_size = JIT_STATE.config.max_bytecode_size;
        let oversized = make_bytecode_of_size(max_size + 100);
        let code = Code::from_bytecode(Bytes::from(oversized));

        let backend = RevmcBackend::default();
        let cache = CodeCache::with_max_entries(64);

        let result = backend.compile_and_cache(&code, Fork::Cancun, &cache);
        assert!(
            result.is_err(),
            "oversized bytecode should fail compilation"
        );

        match result.unwrap_err() {
            JitError::BytecodeTooLarge { size, max } => {
                assert_eq!(size, max_size + 100);
                assert_eq!(max, max_size);
            }
            other => panic!("expected BytecodeTooLarge, got: {other:?}"),
        }

        // Cache should be empty
        assert!(
            cache.get(&(code.hash, Fork::Cancun)).is_none(),
            "oversized bytecode should not be in cache"
        );
    }
}
