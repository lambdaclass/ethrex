//! Arena-based memory lifecycle for JIT-compiled functions.
//!
//! Groups compiled functions into "arenas" — each arena corresponds to one
//! LLVM context/module. When all functions in an arena are evicted from the
//! cache, the entire arena (and its LLVM resources) can be freed.
//!
//! This solves the `std::mem::forget(compiler)` memory leak in `compiler.rs`:
//! instead of leaking every LLVM context, we track which arena each compiled
//! function belongs to and free arenas when they become empty.

use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Unique identifier for an arena (monotonically increasing).
pub type ArenaId = u32;

/// Location of a compiled function: (arena, slot index within that arena).
pub type FuncSlot = (ArenaId, u16);

/// Tracks one arena's live function count.
///
/// An arena is "empty" when `live_count` reaches zero, meaning all its
/// compiled functions have been evicted from the cache and the backing
/// LLVM context can be freed.
#[derive(Debug)]
pub struct ArenaEntry {
    /// Unique arena identifier.
    pub id: ArenaId,
    /// Number of function slots allocated in this arena.
    pub total_slots: u16,
    /// Number of functions still referenced by the cache.
    /// Decremented atomically when a function is evicted.
    live_count: AtomicU32, // u32 for AtomicU32 availability; u16 range enforced by total_slots
}

impl ArenaEntry {
    /// Create a new arena entry with `total_slots` live functions.
    fn new(id: ArenaId, total_slots: u16) -> Self {
        Self {
            id,
            total_slots,
            live_count: AtomicU32::new(u32::from(total_slots)),
        }
    }

    /// Current number of live (non-evicted) functions.
    pub fn live_count(&self) -> u32 {
        self.live_count.load(Ordering::Relaxed)
    }

    /// Decrement live count by 1. Returns `true` if the arena is now empty.
    ///
    /// Uses a CAS loop with saturation at 0 to prevent underflow from
    /// double-evict scenarios.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "underflow guarded by current == 0 check"
    )]
    fn decrement_live(&self) -> bool {
        loop {
            let current = self.live_count.load(Ordering::Relaxed);
            if current == 0 {
                return false; // Already empty, don't underflow
            }
            match self.live_count.compare_exchange_weak(
                current,
                current - 1,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => return current - 1 == 0,
                Err(_) => continue, // Retry on contention
            }
        }
    }
}

/// Statistics snapshot for monitoring arena memory usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaStats {
    /// Number of active arenas (with at least one live function).
    pub active_arenas: usize,
    /// Total live functions across all arenas.
    pub total_live_functions: u64,
    /// Total function slots allocated across all arenas.
    pub total_allocated_slots: u64,
    /// Cumulative count of arenas that have been freed.
    pub arenas_freed: u64,
}

/// Manages the lifecycle of JIT compilation arenas.
///
/// Thread-safe: all methods take `&self` with interior mutability.
/// Lives inside `JitState` and is shared across VM threads.
pub struct ArenaManager {
    /// Active arenas indexed by ID.
    arenas: RwLock<HashMap<ArenaId, ArenaEntry>>,
    /// Next arena ID to allocate (monotonically increasing).
    next_arena_id: AtomicU32,
    /// Default number of function slots per arena.
    pub arena_capacity: u16,
    /// Cumulative count of freed arenas (for stats).
    arenas_freed: AtomicU64,
}

impl ArenaManager {
    /// Create a new arena manager.
    ///
    /// `capacity` is the default number of functions per arena.
    /// Typical value: 64 (balances memory granularity vs. overhead).
    pub fn new(capacity: u16) -> Self {
        Self {
            arenas: RwLock::new(HashMap::new()),
            next_arena_id: AtomicU32::new(0),
            arena_capacity: capacity,
            arenas_freed: AtomicU64::new(0),
        }
    }

    /// Allocate a new arena with the given number of function slots.
    ///
    /// Returns the new arena's ID. The caller is responsible for creating
    /// the corresponding LLVM context (in the compiler thread).
    pub fn allocate_arena(&self, total_slots: u16) -> ArenaId {
        let id = self.next_arena_id.fetch_add(1, Ordering::Relaxed);
        let entry = ArenaEntry::new(id, total_slots);
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        {
            self.arenas.write().unwrap().insert(id, entry);
        }
        id
    }

    /// Mark a function slot as evicted from the cache.
    ///
    /// Returns `true` if this was the last live function in the arena
    /// (meaning the arena's LLVM resources can be freed).
    pub fn mark_evicted(&self, slot: FuncSlot) -> bool {
        let (arena_id, _slot_idx) = slot;
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let arenas = self.arenas.read().unwrap();
        match arenas.get(&arena_id) {
            Some(entry) => entry.decrement_live(),
            None => false, // Arena already removed
        }
    }

    /// Check if an arena has no live functions.
    pub fn is_arena_empty(&self, arena_id: ArenaId) -> bool {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let arenas = self.arenas.read().unwrap();
        match arenas.get(&arena_id) {
            Some(entry) => entry.live_count() == 0,
            None => true, // Already removed
        }
    }

    /// Remove an arena after its LLVM resources have been freed.
    ///
    /// Call this after dropping the `ArenaCompiler` to clean up bookkeeping.
    pub fn remove_arena(&self, arena_id: ArenaId) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let removed = self.arenas.write().unwrap().remove(&arena_id);
        if removed.is_some() {
            self.arenas_freed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get a snapshot of arena statistics.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "accumulator overflow infeasible with u64"
    )]
    pub fn stats(&self) -> ArenaStats {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        let arenas = self.arenas.read().unwrap();
        let mut total_live = 0u64;
        let mut total_allocated = 0u64;
        for entry in arenas.values() {
            total_live += u64::from(entry.live_count());
            total_allocated += u64::from(entry.total_slots);
        }
        ArenaStats {
            active_arenas: arenas.len(),
            total_live_functions: total_live,
            total_allocated_slots: total_allocated,
            arenas_freed: self.arenas_freed.load(Ordering::Relaxed),
        }
    }

    /// Number of active arenas.
    pub fn active_arena_count(&self) -> usize {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        self.arenas.read().unwrap().len()
    }

    /// Reset all state (for test isolation).
    #[cfg(any(test, feature = "test-utils"))]
    pub fn reset(&self) {
        #[expect(clippy::unwrap_used, reason = "RwLock poisoning is unrecoverable")]
        {
            self.arenas.write().unwrap().clear();
        }
        self.next_arena_id.store(0, Ordering::Relaxed);
        self.arenas_freed.store(0, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for ArenaManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.stats();
        f.debug_struct("ArenaManager")
            .field("arena_capacity", &self.arena_capacity)
            .field("active_arenas", &stats.active_arenas)
            .field("total_live_functions", &stats.total_live_functions)
            .field("arenas_freed", &stats.arenas_freed)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_allocate() {
        let mgr = ArenaManager::new(64);
        let id0 = mgr.allocate_arena(4);
        let id1 = mgr.allocate_arena(8);
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
        assert_eq!(mgr.active_arena_count(), 2);
    }

    #[test]
    fn test_arena_mark_evicted() {
        let mgr = ArenaManager::new(64);
        let id = mgr.allocate_arena(3);

        // Evict slot 0 — 2 remaining
        assert!(!mgr.mark_evicted((id, 0)));
        // Evict slot 1 — 1 remaining
        assert!(!mgr.mark_evicted((id, 1)));
        // Evict slot 2 — 0 remaining → true
        assert!(mgr.mark_evicted((id, 2)));
    }

    #[test]
    fn test_arena_full_eviction() {
        let mgr = ArenaManager::new(64);
        let id = mgr.allocate_arena(2);

        assert!(!mgr.is_arena_empty(id));
        assert!(!mgr.mark_evicted((id, 0)));
        assert!(!mgr.is_arena_empty(id));
        assert!(mgr.mark_evicted((id, 1)));
        assert!(mgr.is_arena_empty(id));
    }

    #[test]
    fn test_arena_partial_eviction() {
        let mgr = ArenaManager::new(64);
        let id = mgr.allocate_arena(5);

        mgr.mark_evicted((id, 0));
        mgr.mark_evicted((id, 1));
        mgr.mark_evicted((id, 2));
        // 2 still live
        assert!(!mgr.is_arena_empty(id));
        assert_eq!(mgr.active_arena_count(), 1);
    }

    #[test]
    fn test_arena_double_evict_no_underflow() {
        let mgr = ArenaManager::new(64);
        let id = mgr.allocate_arena(1);

        // First evict → true (arena empty)
        assert!(mgr.mark_evicted((id, 0)));
        // Second evict on same arena → false (no underflow)
        assert!(!mgr.mark_evicted((id, 0)));
        assert!(mgr.is_arena_empty(id));
    }

    #[test]
    fn test_arena_remove() {
        let mgr = ArenaManager::new(64);
        let id = mgr.allocate_arena(1);
        mgr.mark_evicted((id, 0));
        mgr.remove_arena(id);

        assert_eq!(mgr.active_arena_count(), 0);
        assert_eq!(mgr.stats().arenas_freed, 1);
        // is_arena_empty returns true for removed arenas
        assert!(mgr.is_arena_empty(id));
    }

    #[test]
    fn test_arena_remove_nonexistent() {
        let mgr = ArenaManager::new(64);
        // Removing a non-existent arena is a no-op
        mgr.remove_arena(999);
        assert_eq!(mgr.stats().arenas_freed, 0);
    }

    #[test]
    fn test_arena_stats() {
        let mgr = ArenaManager::new(64);

        // Empty manager
        let s = mgr.stats();
        assert_eq!(s.active_arenas, 0);
        assert_eq!(s.total_live_functions, 0);
        assert_eq!(s.total_allocated_slots, 0);
        assert_eq!(s.arenas_freed, 0);

        // Allocate 2 arenas
        let id0 = mgr.allocate_arena(3);
        let _id1 = mgr.allocate_arena(5);

        let s = mgr.stats();
        assert_eq!(s.active_arenas, 2);
        assert_eq!(s.total_live_functions, 8);
        assert_eq!(s.total_allocated_slots, 8);
        assert_eq!(s.arenas_freed, 0);

        // Evict all from arena 0
        mgr.mark_evicted((id0, 0));
        mgr.mark_evicted((id0, 1));
        mgr.mark_evicted((id0, 2));
        mgr.remove_arena(id0);

        let s = mgr.stats();
        assert_eq!(s.active_arenas, 1);
        assert_eq!(s.total_live_functions, 5);
        assert_eq!(s.total_allocated_slots, 5);
        assert_eq!(s.arenas_freed, 1);
    }

    #[test]
    fn test_arena_concurrent_eviction() {
        let mgr = std::sync::Arc::new(ArenaManager::new(64));
        let id = mgr.allocate_arena(100);

        let mut handles = Vec::new();
        for slot_idx in 0..100u16 {
            let mgr_clone = std::sync::Arc::clone(&mgr);
            handles.push(std::thread::spawn(move || {
                mgr_clone.mark_evicted((id, slot_idx))
            }));
        }

        let mut got_true = 0;
        for h in handles {
            if h.join().expect("thread panicked") {
                got_true += 1;
            }
        }

        // Exactly one thread should see the arena become empty
        assert_eq!(got_true, 1, "exactly one thread should return true");
        assert!(mgr.is_arena_empty(id));
    }

    #[test]
    fn test_arena_concurrent_allocate() {
        let mgr = std::sync::Arc::new(ArenaManager::new(64));

        let mut handles = Vec::new();
        for _ in 0..50 {
            let mgr_clone = std::sync::Arc::clone(&mgr);
            handles.push(std::thread::spawn(move || mgr_clone.allocate_arena(4)));
        }

        let mut ids: Vec<ArenaId> = Vec::new();
        for h in handles {
            ids.push(h.join().expect("thread panicked"));
        }

        // All IDs should be unique
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 50, "all arena IDs should be unique");
        assert_eq!(mgr.active_arena_count(), 50);
    }

    #[test]
    fn test_arena_reset() {
        let mgr = ArenaManager::new(64);
        mgr.allocate_arena(4);
        mgr.allocate_arena(4);
        assert_eq!(mgr.active_arena_count(), 2);

        mgr.reset();

        assert_eq!(mgr.active_arena_count(), 0);
        // Next allocation starts from 0 again
        let id = mgr.allocate_arena(1);
        assert_eq!(id, 0);
    }

    #[test]
    fn test_arena_evict_unknown_arena() {
        let mgr = ArenaManager::new(64);
        // Evicting from a non-existent arena is a no-op
        assert!(!mgr.mark_evicted((999, 0)));
    }
}
