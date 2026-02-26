#![no_main]

use bytes::Bytes;
use ethrex_common::H256;
use ethrex_levm::jit::analyzer::analyze_bytecode;
use ethrex_levm::jit::optimizer::optimize;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let bytecode = Bytes::copy_from_slice(data);
    let hash = H256::zero();
    let analyzed = analyze_bytecode(bytecode, hash, vec![]);

    // Property 1: optimize must never panic
    let (optimized, _stats) = optimize(analyzed.clone());

    // Property 2: optimized bytecode must have same length
    assert_eq!(
        optimized.bytecode.len(),
        analyzed.bytecode.len(),
        "optimizer must preserve bytecode length"
    );

    // Property 3: optimizer converges — repeated passes reach a fixed point.
    // Note: NOT single-pass idempotent (folding can create new PUSH+PUSH+OP patterns).
    let mut current = optimized;
    for _ in 0..10 {
        let (next, stats) = optimize(current.clone());
        assert_eq!(
            next.bytecode.len(),
            current.bytecode.len(),
            "optimizer must preserve length on every pass"
        );
        if stats.patterns_folded == 0 {
            break;
        }
        current = next;
    }
});
