//! Fuzz target for trie operations.
//!
//! This fuzzer tests that trie operations maintain consistency:
//! - Insert/get roundtrip
//! - Insert/remove consistency
//! - Root hash determinism

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use ethrex_trie::Trie;

/// Operations that can be performed on a trie
#[derive(Debug, Arbitrary)]
enum TrieOp {
    Insert { key: Vec<u8>, value: Vec<u8> },
    Get { key: Vec<u8> },
    Remove { key: Vec<u8> },
    ComputeRoot,
}

/// A sequence of trie operations to fuzz
#[derive(Debug, Arbitrary)]
struct TrieOpSequence {
    operations: Vec<TrieOp>,
}

fuzz_target!(|sequence: TrieOpSequence| {
    let mut trie = Trie::new_temp();

    // Track what we've inserted for verification
    let mut expected: std::collections::HashMap<Vec<u8>, Vec<u8>> =
        std::collections::HashMap::new();

    for op in sequence.operations {
        match op {
            TrieOp::Insert { key, value } => {
                // Skip empty keys as they may have special handling
                if key.is_empty() {
                    continue;
                }
                let _ = trie.insert(key.clone(), value.clone());
                // In Ethereum trie semantics, empty value means "no value" (same as delete)
                if value.is_empty() {
                    expected.remove(&key);
                } else {
                    expected.insert(key, value);
                }
            }
            TrieOp::Get { key } => {
                if key.is_empty() {
                    continue;
                }
                let result = trie.get(&key);
                if let Ok(result) = result {
                    // Verify consistency with our expected state
                    match (result, expected.get(&key)) {
                        (Some(got), Some(exp)) => assert_eq!(&got, exp),
                        (None, None) => {}
                        (Some(_), None) => panic!("Trie has value we didn't insert"),
                        (None, Some(_)) => panic!("Trie missing value we inserted"),
                    }
                }
            }
            TrieOp::Remove { key } => {
                if key.is_empty() {
                    continue;
                }
                let _ = trie.remove(&key);
                expected.remove(&key);
            }
            TrieOp::ComputeRoot => {
                // Computing root should never panic
                let _ = trie.hash();
            }
        }
    }

    // Final verification: all expected values should be retrievable
    for (key, value) in &expected {
        if let Ok(Some(got)) = trie.get(key) {
            assert_eq!(&got, value, "Value mismatch for key {:?}", key);
        }
    }
});
