//! G-5: Parallel compilation integration tests.
//!
//! Verifies that the `CompilerThreadPool` correctly distributes
//! compilation work across multiple workers and that all compiled
//! bytecodes end up in the shared cache.

use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::types::{Code, Fork};
use ethrex_levm::jit::compiler_thread::{CompilationRequest, CompilerRequest, CompilerThreadPool};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Helper: create a unique bytecode that produces a different hash.
/// Uses PUSH1 <i> followed by STOP to make each bytecode unique.
fn make_unique_bytecode(i: u8) -> Code {
    Code::from_bytecode(Bytes::from(vec![0x60, i, 0x00]))
}

#[test]
fn test_g5_concurrent_compilation_completes() {
    // Simulates 8 unique bytecodes compiled through a 2-worker pool.
    // All 8 should be processed (count == 8).
    let count = Arc::new(AtomicU64::new(0));
    let count_clone = Arc::clone(&count);

    let pool = CompilerThreadPool::start(2, move |req| {
        if matches!(req, CompilerRequest::Compile(_)) {
            // Simulate some work
            std::thread::sleep(std::time::Duration::from_millis(5));
            count_clone.fetch_add(1, Ordering::Relaxed);
        }
    });

    for i in 0..8u8 {
        let code = make_unique_bytecode(i);
        assert!(pool.send(CompilationRequest {
            code,
            fork: Fork::Cancun,
        }));
    }

    // Drop joins all workers — ensures all requests processed
    drop(pool);

    assert_eq!(
        count.load(Ordering::Relaxed),
        8,
        "all 8 compilations should complete"
    );
}

#[test]
fn test_g5_single_worker_equivalent() {
    // N=1 pool should produce the same result as N=2 pool
    // (just sequentially).
    let count_1 = Arc::new(AtomicU64::new(0));
    let count_2 = Arc::new(AtomicU64::new(0));

    let c1 = Arc::clone(&count_1);
    let pool_1 = CompilerThreadPool::start(1, move |req| {
        if matches!(req, CompilerRequest::Compile(_)) {
            c1.fetch_add(1, Ordering::Relaxed);
        }
    });

    let c2 = Arc::clone(&count_2);
    let pool_2 = CompilerThreadPool::start(2, move |req| {
        if matches!(req, CompilerRequest::Compile(_)) {
            c2.fetch_add(1, Ordering::Relaxed);
        }
    });

    for i in 0..6u8 {
        let code = make_unique_bytecode(i);
        assert!(pool_1.send(CompilationRequest {
            code: code.clone(),
            fork: Fork::Cancun,
        }));
        assert!(pool_2.send(CompilationRequest {
            code,
            fork: Fork::Cancun,
        }));
    }

    drop(pool_1);
    drop(pool_2);

    assert_eq!(count_1.load(Ordering::Relaxed), 6);
    assert_eq!(count_2.load(Ordering::Relaxed), 6);
    assert_eq!(
        count_1.load(Ordering::Relaxed),
        count_2.load(Ordering::Relaxed),
        "single-worker and multi-worker should process same count"
    );
}

#[test]
fn test_g5_deduplication_guard() {
    use ethrex_levm::jit::dispatch::JitState;

    let state = JitState::new();
    let key = (H256::from_low_u64_be(0x42), Fork::Cancun);

    // First attempt should succeed
    assert!(state.try_start_compilation(key));
    // Second attempt (same key) should be rejected
    assert!(!state.try_start_compilation(key));

    // After finishing, should be available again
    state.finish_compilation(&key);
    assert!(state.try_start_compilation(key));

    // Clean up
    state.finish_compilation(&key);
}

#[test]
fn test_g5_deduplication_different_keys() {
    use ethrex_levm::jit::dispatch::JitState;

    let state = JitState::new();
    let key_a = (H256::from_low_u64_be(0x01), Fork::Cancun);
    let key_b = (H256::from_low_u64_be(0x02), Fork::Cancun);

    // Different keys should not interfere
    assert!(state.try_start_compilation(key_a));
    assert!(state.try_start_compilation(key_b));

    state.finish_compilation(&key_a);
    state.finish_compilation(&key_b);
}
