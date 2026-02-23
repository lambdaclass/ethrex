//! Execution counter for JIT compilation tiering.
//!
//! Tracks how many times each bytecode (by hash) has been executed.
//! When the count exceeds the compilation threshold, the bytecode
//! becomes a candidate for JIT compilation.

use ethrex_common::H256;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Thread-safe execution counter keyed by bytecode hash.
#[derive(Debug, Clone)]
pub struct ExecutionCounter {
    counts: Arc<RwLock<HashMap<H256, u64>>>,
}

impl ExecutionCounter {
    /// Create a new execution counter.
    pub fn new() -> Self {
        Self {
            counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Increment the execution count for a bytecode hash. Returns the new count.
    pub fn increment(&self, hash: &H256) -> u64 {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut counts = self.counts.write().unwrap();
        let count = counts.entry(*hash).or_insert(0);
        *count = count.saturating_add(1);
        *count
    }

    /// Get the current execution count for a bytecode hash.
    pub fn get(&self, hash: &H256) -> u64 {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let counts = self.counts.read().unwrap();
        counts.get(hash).copied().unwrap_or(0)
    }
}

impl Default for ExecutionCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_increment_and_get() {
        let counter = ExecutionCounter::new();
        let hash = H256::zero();

        assert_eq!(counter.get(&hash), 0);
        assert_eq!(counter.increment(&hash), 1);
        assert_eq!(counter.increment(&hash), 2);
        assert_eq!(counter.get(&hash), 2);
    }

    #[test]
    fn test_distinct_hashes() {
        let counter = ExecutionCounter::new();
        let h1 = H256::zero();
        let h2 = H256::from_low_u64_be(1);

        counter.increment(&h1);
        counter.increment(&h1);
        counter.increment(&h2);

        assert_eq!(counter.get(&h1), 2);
        assert_eq!(counter.get(&h2), 1);
    }
}
