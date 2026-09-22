//! Thread cost of a `Blockchain`'s merkleization pool.
//!
//! Two mechanisms keep it bounded, covering opposite cases, so each gets its own
//! test: the pool is built on first use, so an instance that never merkleizes pays
//! nothing; and `for_test_harness_with_pool` seeds a shared one, for the ef_tests
//! runners where every instance merkleizes and laziness saves nothing.
//!
//! Both matter because rayon only signals a pool's workers on drop and never joins
//! them, so a harness building many short-lived `Blockchain`s accumulates live
//! threads faster than the OS reaps them.
use ethrex_blockchain::{Blockchain, BlockchainOptions};
use ethrex_storage::{EngineType, Store};
use std::sync::Arc;

fn store() -> Store {
    Store::new("", EngineType::InMemory).expect("in-memory store")
}

/// A fresh `Blockchain` must not build its pool before it merkleizes.
#[test]
fn a_new_blockchain_does_not_build_its_merkle_pool_eagerly() {
    let blockchain = Blockchain::new(store(), BlockchainOptions::default());
    assert!(
        !blockchain.merkle_pool_initialized(),
        "constructing a Blockchain built the merkleization pool; only first \
         merkleization should"
    );
}

/// The invariant the ef_tests runners depend on: instances built with a shared pool
/// use *that* pool and never build their own.
///
/// The `strong_count` assertion is the load-bearing half: every instance reporting
/// `merkle_pool_initialized()` would also hold if each had quietly built its own.
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

/// `for_test_harness` disables the mempool prewarmer, whose threads would otherwise
/// outlive the test that created them. `default_with_store` is documented as
/// test-only too, so it must agree.
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
