use bytes::Bytes;
use ethrex_common::Address;
use ethrex_levm::precompiles::{PRECOMPILE_CACHE_MAX_BYTES, PrecompileCache};

const OUTPUT_LEN: usize = 16;

fn output(seed: u8) -> Bytes {
    Bytes::from(vec![seed; OUTPUT_LEN])
}

fn address(seed: u64) -> Address {
    Address::from_low_u64_be(seed)
}

/// One zeroed buffer as large as the budget. Calldata is taken as slices of it, which
/// share the allocation, so filling the real budget costs one allocation per test.
fn buffer() -> Bytes {
    Bytes::from(vec![0; PRECOMPILE_CACHE_MAX_BYTES])
}

/// Calldata whose entry, with an `OUTPUT_LEN` output, is charged exactly half the budget.
/// Entries at different addresses are distinct keys even with the same calldata.
fn half_budget_calldata(buffer: &Bytes) -> Bytes {
    let overhead = PrecompileCache::entry_size(0, OUTPUT_LEN);
    buffer.slice(..PRECOMPILE_CACHE_MAX_BYTES / 2 - overhead)
}

#[test]
fn precompile_cache_stops_caching_once_the_budget_is_full() {
    let buffer = buffer();
    let half = half_budget_calldata(&buffer);
    let cache = PrecompileCache::default();

    cache.insert(address(1), half.clone(), output(1), 1);
    cache.insert(address(2), half.clone(), output(2), 2);
    cache.insert(address(3), Bytes::new(), output(3), 3);

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
    let buffer = buffer();
    let half = half_budget_calldata(&buffer);
    let cache = PrecompileCache::default();

    cache.insert(address(1), half.clone(), output(1), 1);
    cache.insert(address(1), half.clone(), output(1), 1);
    cache.insert(address(2), half.clone(), output(2), 2);

    assert!(cache.get(&address(1), &half).is_some());
    assert!(cache.get(&address(2), &half).is_some());
}

#[test]
fn precompile_cache_skips_an_entry_larger_than_the_budget() {
    let oversized = buffer();
    let cache = PrecompileCache::default();

    cache.insert(address(1), oversized.clone(), output(9), 9);
    cache.insert(address(2), Bytes::new(), output(2), 2);

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
