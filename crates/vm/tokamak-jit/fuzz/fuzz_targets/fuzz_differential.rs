#![no_main]

use libfuzzer_sys::fuzz_target;

// Differential fuzzing: JIT vs interpreter.
// This target requires the revmc-backend feature and LLVM 21.
// It is a placeholder that validates basic properties without LLVM.

fuzz_target!(|data: &[u8]| {
    // Without revmc-backend, we can only validate that the bytecode
    // analysis pipeline doesn't diverge between two passes.
    if data.is_empty() {
        return;
    }

    let bytecode = bytes::Bytes::copy_from_slice(data);
    let hash = ethrex_common::H256::zero();

    let analyzed1 = ethrex_levm::jit::analyzer::analyze_bytecode(bytecode.clone(), hash, vec![]);
    let analyzed2 = ethrex_levm::jit::analyzer::analyze_bytecode(bytecode, hash, vec![]);

    // Determinism check: same input must produce same output
    assert_eq!(analyzed1.basic_blocks, analyzed2.basic_blocks);
    assert_eq!(analyzed1.opcode_count, analyzed2.opcode_count);
    assert_eq!(analyzed1.has_external_calls, analyzed2.has_external_calls);
});
