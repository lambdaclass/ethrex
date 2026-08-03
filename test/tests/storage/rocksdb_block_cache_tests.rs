//! Sizing of the RocksDB shared block cache default.

use ethrex_storage::{
    MAX_ROCKSDB_BLOCK_CACHE_SIZE_BYTES, MIN_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
    ROCKSDB_BLOCK_CACHE_MEMORY_PERCENT, default_rocksdb_block_cache_size,
    rocksdb_block_cache_size_for,
};

const GIB: usize = 1024 * 1024 * 1024;

/// An undetectable memory limit SHALL fall back to the ceiling, preserving the
/// behavior from before the default became memory-aware.
#[test]
fn undetected_memory_limit_falls_back_to_the_ceiling() {
    assert_eq!(
        rocksdb_block_cache_size_for(None),
        MAX_ROCKSDB_BLOCK_CACHE_SIZE_BYTES
    );
}

/// Hosts large enough that the percentage exceeds the ceiling SHALL get the ceiling,
/// so a big machine keeps the tuned 12 GiB rather than scaling without bound.
#[test]
fn large_hosts_are_capped_at_the_ceiling() {
    for limit in [32 * GIB, 64 * GIB, 1024 * GIB] {
        assert_eq!(
            rocksdb_block_cache_size_for(Some(limit)),
            MAX_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
            "{limit} bytes must clamp to the ceiling"
        );
    }
}

/// The case this sizing exists for: on a 16 GiB host the flat 12 GiB default was 71% of
/// the machine, leaving no headroom for trie layers, execution and the mempool. The
/// memory-aware default SHALL leave the majority of such a host to the rest of the node.
#[test]
fn memory_constrained_host_keeps_headroom() {
    let limit = 16 * GIB;
    let cache = rocksdb_block_cache_size_for(Some(limit));

    assert_eq!(cache, limit / 100 * ROCKSDB_BLOCK_CACHE_MEMORY_PERCENT);
    assert!(
        cache < MAX_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
        "a 16 GiB host must not be handed the ceiling"
    );
    assert!(
        limit - cache > limit / 2,
        "more than half of a 16 GiB host must stay available to the node"
    );
}

/// Tiny hosts SHALL still get the floor: below it the per-SST index and filter blocks
/// stop staying resident and every trie read pays an extra seek.
#[test]
fn tiny_hosts_get_the_floor() {
    assert_eq!(
        rocksdb_block_cache_size_for(Some(GIB)),
        MIN_ROCKSDB_BLOCK_CACHE_SIZE_BYTES
    );
    assert_eq!(
        rocksdb_block_cache_size_for(Some(0)),
        MIN_ROCKSDB_BLOCK_CACHE_SIZE_BYTES
    );
}

/// Whatever this machine reports, the resolved default SHALL land inside the clamp.
#[test]
fn resolved_default_is_within_the_clamp() {
    let cache = default_rocksdb_block_cache_size();
    assert!(
        (MIN_ROCKSDB_BLOCK_CACHE_SIZE_BYTES..=MAX_ROCKSDB_BLOCK_CACHE_SIZE_BYTES).contains(&cache),
        "resolved default {cache} outside the clamp"
    );
}
