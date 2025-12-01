// This code is originally from https://github.com/citahub/cita_trie/ (commit: 9a8659f9f40feb3b89868f3964cdfb250f23a1c4),
// licensed under Apache-2. Modified to suit our needs, and to have a baseline to benchmark our own
// trie implementation against an existing one.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use hasher::HasherKeccak;
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

use cita_trie::MemoryDB;
use cita_trie::{PatriciaTrie, Trie};
use ethrex_trie::InMemoryTrieDB as EthrexMemDB;
use ethrex_trie::Trie as EthrexTrie;

#[allow(clippy::unit_arg)]
fn insert_shared_prefix_worst_case_benchmark(c: &mut Criterion) {
    // Keys share the first 28 bytes to have extension nodes
    let (keys, values) = black_box(shared_prefix_data(5000, 28));

    let mut group = c.benchmark_group("Trie shared-prefix worst-case");
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("ethrex-trie insert 5k shared prefix", |b| {
        b.iter_batched_ref(
            || EthrexTrie::new(Box::new(EthrexMemDB::new_empty())),
            |trie| {
                for i in 0..keys.len() {
                    trie.insert(keys[i].clone(), values[i].clone()).unwrap();
                }
                black_box(trie.commit().unwrap());
            },
            criterion::BatchSize::LargeInput,
        );
    });

    group.bench_function("cita-trie insert 5k shared prefix", |b| {
        b.iter_batched_ref(
            || {
                PatriciaTrie::new(
                    Arc::new(MemoryDB::new(false)),
                    Arc::new(HasherKeccak::new()),
                )
            },
            |trie| {
                for i in 0..keys.len() {
                    trie.insert(keys[i].clone(), values[i].clone()).unwrap();
                }
                black_box(trie.root().unwrap());
            },
            criterion::BatchSize::LargeInput,
        );
    });

    group.bench_function("ethrex-trie compute_hash 5k shared prefix", |b| {
        b.iter_batched_ref(
            || {
                let mut trie = EthrexTrie::new(Box::new(EthrexMemDB::new_empty()));
                for i in 0..keys.len() {
                    trie.insert(keys[i].clone(), values[i].clone()).unwrap();
                }
                black_box(trie)
            },
            |trie| black_box(trie.hash_no_commit()),
            criterion::BatchSize::LargeInput,
        );
    });
}

#[allow(clippy::unit_arg)]
fn insert_worse_case_benchmark(c: &mut Criterion) {
    let (keys_1k, values_1k) = black_box(random_data(1000));
    let (keys_10k, values_10k) = black_box(random_data(10000));

    let mut group = c.benchmark_group("Trie random data worst case");
    group.bench_function("ethrex-trie insert 1k", |b| {
        b.iter_batched_ref(
            || EthrexTrie::new(Box::new(EthrexMemDB::new_empty())),
            |trie| {
                for i in 0..keys_1k.len() {
                    trie.insert(
                        black_box(keys_1k[i].clone()),
                        black_box(values_1k[i].clone()),
                    )
                    .unwrap()
                }
                black_box(trie.commit().unwrap());
            },
            criterion::BatchSize::LargeInput,
        );
    });

    group.bench_function("ethrex-trie insert 10k", |b| {
        b.iter_batched_ref(
            || EthrexTrie::new(Box::new(EthrexMemDB::new_empty())),
            |trie| {
                for i in 0..keys_10k.len() {
                    black_box(
                        trie.insert(keys_10k[i].clone(), values_10k[i].clone())
                            .unwrap(),
                    )
                }
                black_box(trie.commit().unwrap());
            },
            criterion::BatchSize::LargeInput,
        );
    });

    group.bench_function("cita-trie insert 1k", |b| {
        b.iter_batched_ref(
            || {
                PatriciaTrie::new(
                    Arc::new(MemoryDB::new(false)),
                    Arc::new(HasherKeccak::new()),
                )
            },
            |trie| {
                for i in 0..keys_1k.len() {
                    trie.insert(
                        black_box(keys_1k[i].clone()),
                        black_box(values_1k[i].clone()),
                    )
                    .unwrap()
                }
                black_box(trie.root().unwrap());
            },
            criterion::BatchSize::LargeInput,
        );
    });

    group.bench_function("cita-trie insert 10k", |b| {
        b.iter_batched_ref(
            || {
                PatriciaTrie::new(
                    Arc::new(MemoryDB::new(false)),
                    Arc::new(HasherKeccak::new()),
                )
            },
            |trie| {
                for i in 0..keys_10k.len() {
                    black_box(
                        trie.insert(keys_10k[i].clone(), values_10k[i].clone())
                            .unwrap(),
                    )
                }
                black_box(trie.root().unwrap());
            },
            criterion::BatchSize::LargeInput,
        );
    });

    group.measurement_time(Duration::from_secs(15));

    group.bench_function("ethrex-trie compute_hash 1k", |b| {
        b.iter_batched_ref(
            || {
                let mut trie = EthrexTrie::new(Box::new(EthrexMemDB::new_empty()));
                for i in 0..keys_1k.len() {
                    trie.insert(keys_1k[i].clone(), values_1k[i].clone())
                        .unwrap();
                }
                black_box(trie)
            },
            |trie| black_box(trie.hash_no_commit()),
            criterion::BatchSize::LargeInput,
        );
    });

    group.bench_function("ethrex-trie compute_hash 10k", |b| {
        b.iter_batched_ref(
            || {
                let mut trie = EthrexTrie::new(Box::new(EthrexMemDB::new_empty()));
                for i in 0..keys_10k.len() {
                    trie.insert(keys_10k[i].clone(), values_10k[i].clone())
                        .unwrap();
                }
                black_box(trie)
            },
            |trie| black_box(trie.hash_no_commit()),
            criterion::BatchSize::LargeInput,
        );
    });
}

fn random_data(n: usize) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut rng = StdRng::seed_from_u64(0xdeadbeef);
    let mut keys = Vec::with_capacity(n);
    let mut values = Vec::with_capacity(n);
    for _ in 0..n {
        let mut k = vec![0u8; 32];
        rng.fill_bytes(&mut k);
        let mut v = vec![0u8; 32];
        rng.fill_bytes(&mut v);
        keys.push(k);
        values.push(v);
    }

    (keys, values)
}

fn shared_prefix_data(n: usize, shared_prefix_bytes: usize) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    assert!(shared_prefix_bytes + 4 <= 32);

    let mut rng = StdRng::seed_from_u64(0xdeadbeef);
    let mut keys = Vec::with_capacity(n);
    let mut values = Vec::with_capacity(n);

    let mut base = [0u8; 32];
    rng.fill_bytes(&mut base);

    for i in 0..n {
        let mut k = base;
        let suffix = (i as u32).to_be_bytes();
        k[shared_prefix_bytes..shared_prefix_bytes + 4].copy_from_slice(&suffix);
        keys.push(k.into());

        let mut v = vec![0u8; 32];
        rng.fill_bytes(&mut v);
        values.push(v);
    }

    (keys, values)
}

fn criterion_config() -> Criterion {
    Criterion::default()
        .sample_size(150)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
}

criterion_group!(
    name = benches;
    config = criterion_config();
    targets =  insert_worse_case_benchmark, insert_shared_prefix_worst_case_benchmark
);
criterion_main!(benches);
