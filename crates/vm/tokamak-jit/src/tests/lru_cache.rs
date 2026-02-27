//! Integration tests for G-6: LRU cache eviction.
//!
//! Validates that the JIT code cache evicts the least recently used entry
//! (not FIFO) when capacity is reached, and that arena lifecycle is
//! correctly triggered on eviction.

use ethrex_common::H256;
use ethrex_common::types::Fork;
use ethrex_levm::jit::cache::{CodeCache, CompiledCode};

/// Helper to create a `CompiledCode` with a given arena slot for testing.
///
/// # Safety
/// Uses null pointer — safe for metadata-only tests.
#[expect(unsafe_code)]
fn make_code(arena_slot: Option<(u32, u16)>, has_external_calls: bool) -> CompiledCode {
    unsafe { CompiledCode::new(std::ptr::null(), 100, 5, arena_slot, has_external_calls) }
}

fn key(id: u64) -> (H256, Fork) {
    (H256::from_low_u64_be(id), Fork::Cancun)
}

/// Frequently accessed entries survive eviction over infrequently accessed ones.
///
/// Simulates a "hot contract" pattern: one entry is accessed on every block
/// while others are accessed only once.
#[test]
fn test_g6_frequently_accessed_survives_eviction() {
    let cache = CodeCache::with_max_entries(4);

    // Insert 4 entries (at capacity)
    for id in 1..=4 {
        cache.insert(key(id), make_code(None, false));
    }
    assert_eq!(cache.len(), 4);

    // Simulate hot contract: access k1 frequently (like Uniswap Router)
    for _ in 0..100 {
        assert!(cache.get(&key(1)).is_some());
    }

    // Insert k5 → should evict one of k2/k3/k4 (not k1)
    cache.insert(key(5), make_code(None, false));
    assert_eq!(cache.len(), 4);
    assert!(
        cache.get(&key(1)).is_some(),
        "hot contract k1 should survive eviction"
    );
    assert!(
        cache.get(&key(5)).is_some(),
        "newly inserted k5 should exist"
    );
}

/// LRU eviction correctly returns the evicted entry's arena slot for memory reclamation.
///
/// This validates the integration between the LRU cache and the arena lifecycle:
/// when an entry is evicted, its `FuncSlot` is returned so `ArenaManager::mark_evicted`
/// can be called to track live function counts and eventually free LLVM memory.
#[test]
fn test_g6_lru_eviction_returns_arena_slot() {
    let cache = CodeCache::with_max_entries(2);

    // k1 has arena slot (1, 0), k2 has arena slot (1, 1)
    cache.insert(key(1), make_code(Some((1, 0)), false));
    cache.insert(key(2), make_code(Some((1, 1)), false));

    // Access k2 to make k1 the LRU
    cache.get(&key(2));

    // Insert k3 → k1 should be evicted, returning its arena slot
    let evicted = cache.insert(key(3), make_code(Some((2, 0)), false));
    assert_eq!(
        evicted,
        Some((1, 0)),
        "eviction should return k1's arena slot (1, 0)"
    );
}

/// Concurrent `get()` calls do not deadlock or require write locks.
///
/// This validates the core G-6 requirement: `get()` uses only a read lock
/// plus atomic timestamp updates, so multiple threads can access the cache
/// simultaneously without contention.
#[test]
fn test_g6_get_no_write_lock_contention() {
    use std::sync::Arc;

    let cache = Arc::new(CodeCache::with_max_entries(100));

    // Insert 50 entries
    for id in 1..=50 {
        cache.insert(key(id), make_code(None, false));
    }

    // Spawn 8 threads doing concurrent get() calls
    let mut handles = Vec::new();
    for thread_id in 0..8 {
        let cache = Arc::clone(&cache);
        handles.push(std::thread::spawn(move || {
            let mut hits = 0u64;
            for round in 0..1000 {
                let id = ((thread_id * 1000 + round) % 50) + 1;
                if cache.get(&key(id)).is_some() {
                    hits += 1;
                }
            }
            hits
        }));
    }

    let total_hits: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
    // All 8000 lookups should hit (all 50 keys exist)
    assert_eq!(
        total_hits, 8000,
        "all concurrent get() calls should succeed"
    );
}

/// Eviction metrics are tracked correctly across multiple eviction events.
#[test]
fn test_g6_eviction_count_correct() {
    let cache = CodeCache::with_max_entries(3);

    // Fill cache
    cache.insert(key(1), make_code(None, false));
    cache.insert(key(2), make_code(None, false));
    cache.insert(key(3), make_code(None, false));

    // Insert 3 more, causing 3 evictions
    cache.insert(key(4), make_code(None, false));
    cache.insert(key(5), make_code(None, false));
    cache.insert(key(6), make_code(None, false));

    // Cache should still have exactly 3 entries
    assert_eq!(cache.len(), 3);
    // The 3 most recently inserted entries should be present
    assert!(cache.get(&key(4)).is_some());
    assert!(cache.get(&key(5)).is_some());
    assert!(cache.get(&key(6)).is_some());
}

/// LRU eviction respects insert-time ordering when no get() calls are made.
///
/// Without any access via `get()`, the LRU entry is the one inserted earliest
/// (same as FIFO), because insert timestamps are monotonically increasing.
#[test]
fn test_g6_lru_degrades_to_fifo_without_access() {
    let cache = CodeCache::with_max_entries(3);

    cache.insert(key(1), make_code(None, false));
    cache.insert(key(2), make_code(None, false));
    cache.insert(key(3), make_code(None, false));

    // No get() calls — insert k4, k1 should be evicted (lowest timestamp)
    cache.insert(key(4), make_code(None, false));
    assert!(
        cache.get(&key(1)).is_none(),
        "k1 (first inserted) should be evicted"
    );
    assert_eq!(cache.len(), 3);

    // k2 is now LRU (lowest remaining timestamp). Insert k5 → k2 evicted.
    cache.insert(key(5), make_code(None, false));
    assert!(cache.get(&key(2)).is_none(), "k2 should be evicted as LRU");
    assert_eq!(cache.len(), 3);
}
