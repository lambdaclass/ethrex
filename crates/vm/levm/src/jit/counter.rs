//! Execution counter for JIT compilation tiering.
//!
//! Tracks how many times each bytecode (by hash) has been executed.
//! When the count exceeds the compilation threshold, the bytecode
//! becomes a candidate for JIT compilation.
//!
//! # Fork assumption
//!
//! The counter is keyed by bytecode hash only (not `(hash, fork)`).
//! This means the compilation threshold fires once per bytecode regardless
//! of fork. This is correct under the assumption that **forks do not change
//! during a node's runtime** — a node runs at a single fork for any given
//! block height. If this assumption is violated (e.g., fork upgrade during
//! live operation), bytecodes compiled for the old fork would not be
//! recompiled for the new fork via the threshold mechanism. The cache
//! lookup (`try_jit_dispatch`) would return `None` for the new fork key,
//! causing a safe fallback to the interpreter.

use ethrex_common::H256;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Thread-safe execution counter keyed by bytecode hash.
///
/// Uses `AtomicU64` values so that `increment()` only needs a read lock
/// for already-seen bytecodes, reducing write-lock contention on the hot path.
#[derive(Debug)]
pub struct ExecutionCounter {
    counts: Arc<RwLock<HashMap<H256, AtomicU64>>>,
}

impl Clone for ExecutionCounter {
    fn clone(&self) -> Self {
        // Clone by reading all atomic values under a read lock
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let guard = self.counts.read().unwrap();
        let cloned: HashMap<H256, AtomicU64> = guard
            .iter()
            .map(|(k, v)| (*k, AtomicU64::new(v.load(Ordering::Relaxed))))
            .collect();
        Self {
            counts: Arc::new(RwLock::new(cloned)),
        }
    }
}

impl ExecutionCounter {
    /// Create a new execution counter.
    pub fn new() -> Self {
        Self {
            counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Increment the execution count for a bytecode hash. Returns the new count.
    ///
    /// Fast path: read lock + atomic fetch_add for already-seen bytecodes.
    /// Slow path: write lock for first-seen bytecodes (double-check after upgrade).
    pub fn increment(&self, hash: &H256) -> u64 {
        // Fast path: try read lock first
        {
            #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
            let counts = self.counts.read().unwrap();
            if let Some(counter) = counts.get(hash) {
                return counter.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            }
        }

        // Slow path: take write lock for first-seen bytecode
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut counts = self.counts.write().unwrap();
        // Double-check: another thread may have inserted between read→write upgrade
        if let Some(counter) = counts.get(hash) {
            return counter.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        }
        counts.insert(*hash, AtomicU64::new(1));
        1
    }

    /// Remove all execution counts.
    ///
    /// Used by `JitState::reset_for_testing()` to prevent state leakage
    /// between `#[serial]` tests. Not available in production builds.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn clear(&self) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let mut counts = self.counts.write().unwrap();
        counts.clear();
    }

    /// Get the current execution count for a bytecode hash.
    pub fn get(&self, hash: &H256) -> u64 {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let counts = self.counts.read().unwrap();
        counts
            .get(hash)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
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
    fn test_clear() {
        let counter = ExecutionCounter::new();
        let h1 = H256::zero();
        let h2 = H256::from_low_u64_be(1);

        counter.increment(&h1);
        counter.increment(&h1);
        counter.increment(&h2);
        assert_eq!(counter.get(&h1), 2);
        assert_eq!(counter.get(&h2), 1);

        counter.clear();
        assert_eq!(counter.get(&h1), 0);
        assert_eq!(counter.get(&h2), 0);
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
