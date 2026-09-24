use bytes::Bytes;
use ethrex_common::Address;
use ethrex_levm::precompiles::PrecompileCache;

const PAYLOAD_LEN: usize = 16;

fn payload(seed: u8) -> Bytes {
    Bytes::from(vec![seed; PAYLOAD_LEN])
}

fn address(seed: u64) -> Address {
    Address::from_low_u64_be(seed)
}

/// Room for exactly `n` entries of `PAYLOAD_LEN`-byte calldata and output.
fn budget_for(n: usize) -> usize {
    PrecompileCache::entry_size(PAYLOAD_LEN, PAYLOAD_LEN).saturating_mul(n)
}

#[test]
fn precompile_cache_stops_caching_once_the_budget_is_full() {
    let cache = PrecompileCache::with_max_bytes(budget_for(2));

    cache.insert(address(1), payload(1), payload(1), 1);
    cache.insert(address(2), payload(2), payload(2), 2);
    cache.insert(address(3), payload(3), payload(3), 3);

    // Entries cached before the budget filled stay available for the rest of the block.
    assert_eq!(cache.get(&address(1), &payload(1)), Some((payload(1), 1)));
    assert_eq!(cache.get(&address(2), &payload(2)), Some((payload(2), 2)));
    // The one that would have gone over is not cached; callers just recompute it.
    assert_eq!(cache.get(&address(3), &payload(3)), None);
}

/// The warmer and the executor can both insert the same call. Charging it twice would
/// fill the budget early and turn away results that fit.
#[test]
fn precompile_cache_charges_a_repeated_key_once() {
    let cache = PrecompileCache::with_max_bytes(budget_for(2));

    cache.insert(address(1), payload(1), payload(1), 1);
    cache.insert(address(1), payload(1), payload(1), 1);
    cache.insert(address(2), payload(2), payload(2), 2);

    assert!(cache.get(&address(1), &payload(1)).is_some());
    assert!(cache.get(&address(2), &payload(2)).is_some());
}

#[test]
fn precompile_cache_skips_an_entry_larger_than_the_budget() {
    let budget = budget_for(2);
    let cache = PrecompileCache::with_max_bytes(budget);

    let oversized = Bytes::from(vec![9; budget]);
    cache.insert(address(1), oversized.clone(), payload(9), 9);
    cache.insert(address(2), payload(2), payload(2), 2);

    assert_eq!(cache.get(&address(1), &oversized), None);
    // Skipping the oversized entry must not use up any of the budget.
    assert!(cache.get(&address(2), &payload(2)).is_some());
}

#[test]
fn precompile_cache_can_be_disabled_with_zero_budget() {
    let cache = PrecompileCache::with_max_bytes(0);

    cache.insert(address(1), payload(5), payload(5), 5);

    assert_eq!(cache.get(&address(1), &payload(5)), None);
}

/// The budget counts each entry's fixed storage, not only its payload, so many tiny
/// entries cannot hold far more memory than the budget says.
#[test]
fn precompile_cache_charges_fixed_overhead_per_entry() {
    assert!(PrecompileCache::entry_size(0, 0) > 0);
    assert!(PrecompileCache::entry_size(10, 20) > 30);
}
