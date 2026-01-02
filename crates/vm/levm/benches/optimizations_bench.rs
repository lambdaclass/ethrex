//! Micro-benchmarks for the optimization changes
//! Run with: cargo bench -p ethrex-levm --bench optimizations_bench

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use rustc_hash::FxHashSet;

/// Benchmark JUMP target lookup: HashSet vs Vec binary search
fn bench_jump_target_lookup(c: &mut Criterion) {
    // Simulate a typical contract with ~100 JUMPDEST instructions
    let jump_targets_vec: Vec<u32> = (0..100).map(|i| i * 50).collect();
    let jump_targets_set: FxHashSet<u32> = jump_targets_vec.iter().copied().collect();

    // Target that exists (middle of the list)
    let existing_target = 2500u32;
    // Target that doesn't exist
    let missing_target = 2501u32;

    let mut group = c.benchmark_group("jump_target_lookup");

    group.bench_function("vec_binary_search_hit", |b| {
        b.iter(|| {
            black_box(jump_targets_vec.binary_search(&existing_target).is_ok())
        })
    });

    group.bench_function("hashset_contains_hit", |b| {
        b.iter(|| {
            black_box(jump_targets_set.contains(&existing_target))
        })
    });

    group.bench_function("vec_binary_search_miss", |b| {
        b.iter(|| {
            black_box(jump_targets_vec.binary_search(&missing_target).is_ok())
        })
    });

    group.bench_function("hashset_contains_miss", |b| {
        b.iter(|| {
            black_box(jump_targets_set.contains(&missing_target))
        })
    });

    group.finish();
}

/// Benchmark precompile address check
fn bench_precompile_check(c: &mut Criterion) {
    use ethrex_common::Address;

    // Precompile address (ECRECOVER = 0x01)
    let precompile_addr = Address::from_low_u64_be(1);
    // Non-precompile address (typical contract)
    let contract_addr = Address::from_low_u64_be(0x1234567890abcdef);

    let mut group = c.benchmark_group("precompile_check");

    // Test the fast path (first 18 bytes check)
    group.bench_function("non_precompile_fast_path", |b| {
        b.iter(|| {
            // Fast path: first 18 bytes != 0
            black_box(contract_addr[0..18] != [0u8; 18])
        })
    });

    group.bench_function("precompile_full_check", |b| {
        b.iter(|| {
            // Full check for precompile
            let addr = &precompile_addr;
            if addr[0..18] != [0u8; 18] {
                black_box(false)
            } else {
                let index = u16::from_be_bytes([addr[18], addr[19]]) as usize;
                black_box(index == 1) // ECRECOVER
            }
        })
    });

    group.finish();
}

/// Benchmark storage slot tracking: FxHashSet vs BTreeSet
fn bench_storage_slot_tracking(c: &mut Criterion) {
    use std::collections::BTreeSet;
    use ethrex_common::H256;

    // Simulate accessing 50 storage slots
    let slots: Vec<H256> = (0..50).map(|i| {
        let mut bytes = [0u8; 32];
        bytes[31] = i as u8;
        H256(bytes)
    }).collect();

    let mut group = c.benchmark_group("storage_slot_tracking");

    group.bench_function("btreeset_insert_50", |b| {
        b.iter(|| {
            let mut set = BTreeSet::new();
            for slot in &slots {
                black_box(set.insert(*slot));
            }
            black_box(set)
        })
    });

    group.bench_function("fxhashset_insert_50", |b| {
        b.iter(|| {
            let mut set = FxHashSet::default();
            for slot in &slots {
                black_box(set.insert(*slot));
            }
            black_box(set)
        })
    });

    // Lookup after population
    let btree_set: BTreeSet<H256> = slots.iter().copied().collect();
    let fx_set: FxHashSet<H256> = slots.iter().copied().collect();
    let lookup_slot = slots[25];

    group.bench_function("btreeset_contains", |b| {
        b.iter(|| {
            black_box(btree_set.contains(&lookup_slot))
        })
    });

    group.bench_function("fxhashset_contains", |b| {
        b.iter(|| {
            black_box(fx_set.contains(&lookup_slot))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_jump_target_lookup,
    bench_precompile_check,
    bench_storage_slot_tracking,
);
criterion_main!(benches);
