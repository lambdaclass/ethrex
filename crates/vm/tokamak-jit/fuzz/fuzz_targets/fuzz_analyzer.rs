#![no_main]

use bytes::Bytes;
use ethrex_common::H256;
use ethrex_levm::jit::analyzer::analyze_bytecode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Feed arbitrary bytes as EVM bytecode
    let bytecode = Bytes::copy_from_slice(data);
    let hash = H256::zero();
    let jump_targets = vec![];

    // Property 1: analyze_bytecode must never panic
    let analyzed = analyze_bytecode(bytecode, hash, jump_targets);

    // Property 2: basic block boundaries must be within bytecode bounds
    for (start, end) in &analyzed.basic_blocks {
        assert!(*start <= *end, "block start must be <= end");
        assert!(
            *end < analyzed.bytecode.len(),
            "block end must be within bytecode"
        );
    }

    // Property 3: opcode_count must be <= bytecode length
    assert!(
        analyzed.opcode_count <= analyzed.bytecode.len(),
        "opcode_count ({}) must be <= bytecode length ({})",
        analyzed.opcode_count,
        analyzed.bytecode.len()
    );
});
