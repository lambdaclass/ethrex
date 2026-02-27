//! Property-based tests for bytecode analysis and optimization.
//!
//! Uses proptest to verify invariants that must hold for all valid
//! and invalid EVM bytecodes.

use bytes::Bytes;
use ethrex_common::H256;
use ethrex_levm::jit::analyzer::analyze_bytecode;
use ethrex_levm::jit::optimizer::optimize;
use proptest::prelude::*;

/// Generate arbitrary bytecode of varying lengths.
fn arb_bytecode() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..1024)
}

proptest! {
    /// The analyzer must never panic on any byte sequence.
    #[test]
    fn analyzer_never_panics(bytecode in arb_bytecode()) {
        let _ = analyze_bytecode(
            Bytes::from(bytecode),
            H256::zero(),
            vec![],
        );
    }

    /// Basic block boundaries must always be within bytecode bounds.
    #[test]
    fn basic_blocks_within_bounds(bytecode in arb_bytecode()) {
        let analyzed = analyze_bytecode(
            Bytes::from(bytecode),
            H256::zero(),
            vec![],
        );
        for (start, end) in &analyzed.basic_blocks {
            prop_assert!(*start <= *end, "block start {} > end {}", start, end);
            prop_assert!(
                *end < analyzed.bytecode.len(),
                "block end {} >= bytecode length {}",
                end,
                analyzed.bytecode.len()
            );
        }
    }

    /// The optimizer must preserve bytecode length (same-size rewriting).
    #[test]
    fn optimizer_preserves_length(bytecode in arb_bytecode()) {
        let original_len = bytecode.len();
        let analyzed = analyze_bytecode(
            Bytes::from(bytecode),
            H256::zero(),
            vec![],
        );
        let (optimized, _stats) = optimize(analyzed);
        prop_assert_eq!(
            optimized.bytecode.len(),
            original_len,
            "optimizer changed bytecode length"
        );
    }

    /// The optimizer converges: repeated passes eventually reach a fixed point
    /// where no further folding occurs (bytecode stabilizes).
    ///
    /// Note: the optimizer is NOT single-pass idempotent because folding
    /// `PUSH+PUSH+OP` can create new adjacent `PUSH+PUSH+OP` patterns.
    /// However, it must converge within a bounded number of passes.
    #[test]
    fn optimizer_converges(bytecode in arb_bytecode()) {
        let analyzed = analyze_bytecode(
            Bytes::from(bytecode),
            H256::zero(),
            vec![],
        );

        // Run up to 10 passes — must converge
        let mut current = analyzed;
        for pass in 0..10 {
            let (next, stats) = optimize(current.clone());
            // Length must always be preserved
            prop_assert_eq!(
                next.bytecode.len(),
                current.bytecode.len(),
                "pass {} changed bytecode length",
                pass
            );
            if stats.patterns_folded == 0 && stats.unary_patterns_folded == 0 {
                // Reached fixed point — verify truly stable
                let (final_check, final_stats) = optimize(next.clone());
                prop_assert_eq!(
                    final_check.bytecode.as_ref(),
                    next.bytecode.as_ref(),
                    "not stable after convergence at pass {}",
                    pass
                );
                prop_assert_eq!(final_stats.patterns_folded, 0);
                prop_assert_eq!(final_stats.unary_patterns_folded, 0);
                return Ok(());
            }
            current = next;
        }
        // If we didn't converge in 10 passes, that's a bug
        prop_assert!(false, "optimizer did not converge in 10 passes");
    }
}
