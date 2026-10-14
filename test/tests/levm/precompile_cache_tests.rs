use bytes::Bytes;
use ethrex_common::Address;
use ethrex_levm::precompiles::PrecompileCache;

const OUTPUT_LEN: usize = 16;
/// The caches here hold this much rather than the production budget, so filling one costs
/// a few KiB instead of copying tens of MiB.
const BUDGET: usize = 4096;

fn output(seed: u8) -> Bytes {
    Bytes::from(vec![seed; OUTPUT_LEN])
}

fn address(seed: u64) -> Address {
    Address::from_low_u64_be(seed)
}

fn cache() -> PrecompileCache {
    PrecompileCache::with_max_bytes(BUDGET)
}

/// Calldata whose entry, with an `OUTPUT_LEN` output, is charged exactly half the budget.
/// Entries at different addresses are distinct keys even with the same calldata.
fn half_budget_calldata() -> Bytes {
    let overhead = PrecompileCache::entry_size(0, OUTPUT_LEN);
    Bytes::from(vec![0; BUDGET / 2 - overhead])
}

#[test]
fn precompile_cache_stops_caching_once_the_budget_is_full() {
    let half = half_budget_calldata();
    let cache = cache();

    cache.insert(address(1), &half, &output(1), 1);
    cache.insert(address(2), &half, &output(2), 2);
    cache.insert(address(3), &[], &output(3), 3);

    // Entries cached before the budget filled stay available for the rest of the block.
    assert_eq!(cache.get(&address(1), &half), Some((output(1), 1)));
    assert_eq!(cache.get(&address(2), &half), Some((output(2), 2)));
    // Even a tiny entry is turned away once the budget is full; callers just recompute it.
    assert_eq!(cache.get(&address(3), &Bytes::new()), None);
}

/// The warmer and the executor can both insert the same call. Charging it twice would
/// fill the budget early and turn away results that fit.
#[test]
fn precompile_cache_charges_a_repeated_key_once() {
    let half = half_budget_calldata();
    let cache = cache();

    cache.insert(address(1), &half, &output(1), 1);
    cache.insert(address(1), &half, &output(1), 1);
    cache.insert(address(2), &half, &output(2), 2);

    assert!(cache.get(&address(1), &half).is_some());
    assert!(cache.get(&address(2), &half).is_some());
}

#[test]
fn precompile_cache_skips_an_entry_larger_than_the_budget() {
    let oversized = Bytes::from(vec![0; BUDGET]);
    let cache = cache();

    cache.insert(address(1), &oversized, &output(9), 9);
    cache.insert(address(2), &[], &output(2), 2);

    assert_eq!(cache.get(&address(1), &oversized), None);
    // Skipping the oversized entry must not use up any of the budget.
    assert!(cache.get(&address(2), &Bytes::new()).is_some());
}

/// The budget counts each entry's fixed storage, not only its payload, so many tiny
/// entries cannot hold far more memory than the budget says.
#[test]
fn precompile_cache_charges_fixed_overhead_per_entry() {
    assert!(PrecompileCache::entry_size(0, 0) > 0);
    assert!(PrecompileCache::entry_size(10, 20) > 30);
}

/// Calldata can be a view of a larger allocation, such as a reused call buffer. A cached
/// entry must not keep that allocation alive, since the budget counts only the length.
#[test]
fn precompile_cache_keeps_no_reference_to_the_callers_buffers() {
    let calldata = Bytes::from(vec![7; 64]);
    let result = output(9);
    let cache = cache();

    cache.insert(address(1), &calldata, &result, 1);

    assert_eq!(cache.get(&address(1), &calldata), Some((result.clone(), 1)));
    assert!(
        calldata.try_into_mut().is_ok(),
        "the cache shares the calldata buffer"
    );
    assert!(
        result.try_into_mut().is_ok(),
        "the cache shares the output buffer"
    );
}

/// The tests above use a small budget. The cache block execution builds uses the production
/// one, which a 1 MiB result fits in with room to spare.
#[test]
fn a_default_precompile_cache_keeps_a_result_far_over_the_test_budget() {
    let calldata = Bytes::from(vec![0; 1024 * 1024]);
    let cache = PrecompileCache::new();

    cache.insert(address(1), &calldata, &output(1), 1);

    assert!(cache.get(&address(1), &calldata).is_some());
}
