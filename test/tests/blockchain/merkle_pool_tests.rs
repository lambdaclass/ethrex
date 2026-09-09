//! Thread cost of a `Blockchain`'s merkleization pool.
//!
//! The pool is 17 OS threads, and `rayon::ThreadPool`'s `Drop` only signals its
//! workers to terminate — it never joins them. A harness that builds many
//! short-lived `Blockchain`s therefore accumulates un-reaped threads, and enough of
//! that backlog crosses the macOS CI runner's per-task thread cap and aborts the
//! whole test binary with SIGABRT before libtest can report anything.
//!
//! Two mechanisms keep the cost bounded, and each needs its own test because they
//! cover opposite cases:
//!
//! - `Blockchain::new` / `for_test_harness` build the pool on *first use*, so an
//!   instance that never merkleizes costs nothing. That covers read-only RPC, which
//!   is nearly every test in this binary.
//! - `for_test_harness_with_pool` seeds a *shared* pool, for harnesses where every
//!   instance merkleizes and laziness therefore buys nothing. The ef_tests runners
//!   are all of this shape: one `Blockchain` per fixture, `add_block_pipeline`
//!   called on it immediately, across ~10k+ blockchain and ~5600 engine fixtures.

use ethrex_blockchain::{Blockchain, BlockchainOptions};
use ethrex_storage::{EngineType, Store};
use std::sync::Arc;

fn store() -> Store {
    Store::new("", EngineType::InMemory).expect("in-memory store")
}

/// A fresh `Blockchain` must not pay for 17 threads before it merkleizes.
#[test]
fn a_new_blockchain_does_not_build_its_merkle_pool_eagerly() {
    let blockchain = Blockchain::new(store(), BlockchainOptions::default());
    assert!(
        !blockchain.merkle_pool_initialized(),
        "constructing a Blockchain built the 17-thread merkleization pool; only \
         first merkleization should"
    );
}

/// The invariant the ef_tests runners depend on: instances built with a shared pool
/// use *that* pool and never build their own.
///
/// Without this, each of the ~10k+ fixtures spawns its own 17 `merkle-worker`
/// threads. The `strong_count` assertion is the load-bearing half — every instance
/// reporting `merkle_pool_initialized()` would also be true if each had quietly
/// built a pool of its own.
#[test]
fn a_shared_pool_is_used_by_every_instance_that_seeds_from_it() {
    const INSTANCES: usize = 8;

    let pool = Blockchain::build_merkle_pool();
    let baseline = Arc::strong_count(&pool);

    let blockchains: Vec<Blockchain> = (0..INSTANCES)
        .map(|_| Blockchain::for_test_harness_with_pool(store(), pool.clone()))
        .collect();

    assert_eq!(
        Arc::strong_count(&pool),
        baseline + INSTANCES,
        "each instance must hold the shared pool, not one of its own"
    );
    assert!(
        blockchains.iter().all(Blockchain::merkle_pool_initialized),
        "a seeded instance must report its pool as already built, so nothing can \
         lazily build a second one"
    );

    // Dropping them releases the shared pool back to this test's own reference,
    // which is what lets a runner keep one pool alive across every fixture.
    drop(blockchains);
    assert_eq!(Arc::strong_count(&pool), baseline);
}

/// `for_test_harness` disables the mempool prewarmer, whose OS thread plus rayon
/// pool (half the available cores) would otherwise outlive the test that created it.
/// `default_with_store` is documented as test-only too, so it must agree.
#[test]
fn test_constructors_disable_the_mempool_prewarmer() {
    assert!(
        !Blockchain::for_test_harness(store())
            .options
            .mempool_prewarm_enabled,
        "for_test_harness must not leave the prewarmer enabled"
    );
    assert!(
        !Blockchain::default_with_store(store())
            .options
            .mempool_prewarm_enabled,
        "default_with_store is a test-only constructor, so it must disable the \
         prewarmer like for_test_harness does"
    );
    assert!(
        BlockchainOptions::default().mempool_prewarm_enabled,
        "the default must stay enabled: production reads its options from the CLI \
         and relies on this default being on"
    );
}
