use bytes::Bytes;
use ethrex_common::Address;
use ethrex_levm::precompiles::PrecompileCache;
use std::mem::size_of;

const ENTRY_METADATA_BYTES: usize =
    size_of::<Address>() + (2 * size_of::<Bytes>()) + size_of::<u64>();

fn entry_size(calldata_len: usize, output_len: usize) -> usize {
    ENTRY_METADATA_BYTES
        .saturating_add(calldata_len)
        .saturating_add(output_len)
}

fn sample_calldata(seed: u8) -> Bytes {
    Bytes::from(vec![seed; 16])
}

fn sample_output(seed: u8) -> Bytes {
    Bytes::from(vec![seed; 16])
}

#[test]
fn precompile_cache_evicts_lru_entry_when_max_size_is_reached() {
    let capacity = entry_size(16, 16).saturating_mul(2);
    let cache = PrecompileCache::with_max_bytes(capacity);

    let address_a = Address::from_low_u64_be(1);
    let address_b = Address::from_low_u64_be(2);
    let address_c = Address::from_low_u64_be(3);

    let calldata_a = sample_calldata(1);
    let calldata_b = sample_calldata(2);
    let calldata_c = sample_calldata(3);

    cache.insert(address_a, calldata_a.clone(), sample_output(1), 1);
    cache.insert(address_b, calldata_b.clone(), sample_output(2), 2);

    // Touch A so B becomes LRU.
    assert!(cache.get(&address_a, &calldata_a).is_some());

    // Third insert should evict B.
    cache.insert(address_c, calldata_c.clone(), sample_output(3), 3);

    assert!(cache.get(&address_a, &calldata_a).is_some());
    assert!(cache.get(&address_b, &calldata_b).is_none());
    assert!(cache.get(&address_c, &calldata_c).is_some());
}

#[test]
fn precompile_cache_skips_entries_larger_than_capacity() {
    let capacity = entry_size(16, 16).saturating_mul(2);
    let cache = PrecompileCache::with_max_bytes(capacity);

    let small_address = Address::from_low_u64_be(1);
    let small_calldata = sample_calldata(7);
    cache.insert(small_address, small_calldata.clone(), sample_output(7), 7);

    let huge_address = Address::from_low_u64_be(2);
    let huge_calldata = Bytes::from(vec![9; capacity]);
    let huge_output = sample_output(9);
    cache.insert(huge_address, huge_calldata.clone(), huge_output, 9);

    assert!(cache.get(&small_address, &small_calldata).is_some());
    assert!(cache.get(&huge_address, &huge_calldata).is_none());
}

#[test]
fn precompile_cache_can_be_disabled_with_zero_capacity() {
    let cache = PrecompileCache::with_max_bytes(0);
    let address = Address::from_low_u64_be(1);
    let calldata = sample_calldata(5);

    cache.insert(address, calldata.clone(), sample_output(5), 5);

    assert!(cache.get(&address, &calldata).is_none());
}
