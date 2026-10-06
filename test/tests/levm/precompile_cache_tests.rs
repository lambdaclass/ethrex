use bytes::Bytes;
use ethrex_common::{Address, U256, types::Fork};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::precompiles::{
    ECADD, ECMUL, ECRECOVER, MODEXP, PRECOMPILE_CACHE_MAX_BYTES, PrecompileCache, SHA2_256,
    execute_precompile,
};

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

const SENTINEL_GAS: u64 = 7;

/// An output no precompile produces, so getting it back proves the call was served
/// from the cache entry planted under `key`.
fn sentinel() -> Bytes {
    Bytes::from_static(b"served from the cache")
}

fn run(address: Address, calldata: &[u8], cache: &PrecompileCache) -> (Bytes, u64) {
    const GAS: u64 = 1_000_000;
    let mut gas_remaining = GAS;
    let output = execute_precompile(
        address,
        &Bytes::copy_from_slice(calldata),
        &mut gas_remaining,
        Fork::Prague,
        Some(cache),
        &NativeCrypto,
        None,
    )
    .expect("precompile call succeeds");
    (output, GAS - gas_remaining)
}

fn with_trailing_bytes(calldata: &[u8]) -> Vec<u8> {
    [calldata, &[0xaa; 40]].concat()
}

/// ECRECOVER, ECADD and ECMUL read a fixed-length prefix: bytes after it must not
/// make an otherwise identical call miss.
#[test]
fn precompile_cache_ignores_bytes_past_a_fixed_size_input() {
    for (precompile, read) in [(ECRECOVER, 128), (ECADD, 128), (ECMUL, 96)] {
        let cache = PrecompileCache::default();
        let key = vec![0; read];
        cache.insert(
            precompile.address,
            key.clone().into(),
            sentinel(),
            SENTINEL_GAS,
        );

        assert_eq!(
            run(precompile.address, &with_trailing_bytes(&key), &cache),
            (sentinel(), SENTINEL_GAS)
        );
    }
}

/// The entry is stored under the bytes that were read, so it is no larger than them
/// and a later call with different trailing bytes finds it.
#[test]
fn precompile_cache_stores_the_significant_prefix() {
    let cache = PrecompileCache::default();
    let key = vec![0; 128];
    let calldata = with_trailing_bytes(&key);

    let computed = run(ECADD.address, &calldata, &cache);

    assert_eq!(cache.get(&ECADD.address, &key.into()), Some(computed));
    assert_eq!(cache.get(&ECADD.address, &calldata.into()), None);
}

/// Dropping the ignored bytes must not change what a call returns or costs.
#[test]
fn precompile_cache_prefix_key_preserves_results() {
    // 3 ** 5 mod 7 == 5, with the lengths in the 96-byte header.
    let mut modexp = Vec::new();
    for size in [1u64, 1, 1] {
        modexp.extend_from_slice(&U256::from(size).to_big_endian());
    }
    modexp.extend_from_slice(&[3, 5, 7]);

    for (address, calldata) in [
        (ECRECOVER.address, vec![0; 128]),
        (ECADD.address, vec![0; 128]),
        (ECMUL.address, vec![0; 96]),
        (MODEXP.address, modexp),
    ] {
        let uncached = run(address, &calldata, &PrecompileCache::default());
        let cache = PrecompileCache::default();
        // The first call fills the cache, the second one is served from it.
        assert_eq!(
            run(address, &with_trailing_bytes(&calldata), &cache),
            uncached
        );
        assert_eq!(
            run(address, &with_trailing_bytes(&calldata), &cache),
            uncached
        );
        assert_eq!(run(address, &calldata, &cache), uncached);
    }
}

/// MODEXP reads what its header announces and nothing after it.
#[test]
fn precompile_cache_follows_the_modexp_header() {
    let mut key = Vec::new();
    for size in [1u64, 1, 1] {
        key.extend_from_slice(&U256::from(size).to_big_endian());
    }
    key.extend_from_slice(&[3, 5, 7]);

    let cache = PrecompileCache::default();
    cache.insert(MODEXP.address, key.clone().into(), sentinel(), SENTINEL_GAS);

    assert_eq!(
        run(MODEXP.address, &with_trailing_bytes(&key), &cache),
        (sentinel(), SENTINEL_GAS)
    );

    // The exponent is part of what MODEXP reads: a different one is a different call.
    let mut other_exponent = key.clone();
    other_exponent[97] = 6;
    assert_eq!(
        run(MODEXP.address, &other_exponent, &cache).0,
        Bytes::from_static(&[1])
    );
}

/// SHA-256 hashes its whole input, so every byte stays part of the key.
#[test]
fn precompile_cache_keeps_the_whole_input_when_all_of_it_is_read() {
    let cache = PrecompileCache::default();
    let key = vec![0; 128];
    cache.insert(
        SHA2_256.address,
        key.clone().into(),
        sentinel(),
        SENTINEL_GAS,
    );

    let (output, _) = run(SHA2_256.address, &with_trailing_bytes(&key), &cache);

    assert_ne!(output, sentinel());
    assert_eq!(output.len(), 32);
}
