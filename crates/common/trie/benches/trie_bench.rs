// This code is originally from https://github.com/citahub/cita_trie/ (commit: 9a8659f9f40feb3b89868f3964cdfb250f23a1c4),
// licensed under Apache-2. Modified to suit our needs, and to have a baseline to benchmark our own
// trie implementation against an existing one.

use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use ethereum_types::H256;
use hasher::HasherKeccak;

use cita_trie::MemoryDB;
use cita_trie::{PatriciaTrie, Trie};
use ethrex_trie::InMemoryTrieDB as EthrexMemDB;
use ethrex_trie::Trie as EthrexTrie;

#[allow(clippy::unit_arg)]
fn insert_worse_case_benchmark(c: &mut Criterion) {
    let (keys_1k, values_1k) = random_data(1000);
    let (keys_10k, values_10k) = random_data(10000);

    let mut group = c.benchmark_group("Trie");

    group.bench_function("ethrex-trie insert 1k", |b| {
        let mut trie = EthrexTrie::new(Box::new(EthrexMemDB::new_empty()));
        b.iter(|| {
            for i in 0..keys_1k.len() {
                black_box(&mut trie)
                    .insert(
                        black_box(keys_1k[i].clone()),
                        black_box(values_1k[i].clone()),
                    )
                    .unwrap()
            }
            trie.commit().unwrap();
        });
    });

    group.bench_function("ethrex-trie insert 10k", |b| {
        let mut trie = EthrexTrie::new(Box::new(EthrexMemDB::new_empty()));

        b.iter(|| {
            for i in 0..keys_10k.len() {
                black_box(
                    trie.insert(keys_10k[i].clone(), values_10k[i].clone())
                        .unwrap(),
                )
            }
            black_box(trie.commit().unwrap());
        });
    });

    group.bench_function("cita-trie insert 1k", |b| {
        let mut trie = PatriciaTrie::new(
            Arc::new(MemoryDB::new(false)),
            Arc::new(HasherKeccak::new()),
        );

        b.iter(|| {
            for i in 0..keys_1k.len() {
                black_box(&mut trie)
                    .insert(
                        black_box(keys_1k[i].clone()),
                        black_box(values_1k[i].clone()),
                    )
                    .unwrap()
            }
            trie.root().unwrap()
        });
    });

    group.bench_function("cita-trie insert 10k", |b| {
        let mut trie = PatriciaTrie::new(
            Arc::new(MemoryDB::new(false)),
            Arc::new(HasherKeccak::new()),
        );

        b.iter(|| {
            for i in 0..keys_10k.len() {
                black_box(
                    trie.insert(keys_10k[i].clone(), values_10k[i].clone())
                        .unwrap(),
                )
            }
            trie.root().unwrap()
        });
    });
}

fn random_data(n: usize) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut keys = Vec::with_capacity(n);
    let mut values = Vec::with_capacity(n);
    for _ in 0..n {
        let key = H256::random().to_fixed_bytes().into();
        let value = H256::random().to_fixed_bytes().into();
        keys.push(key);
        values.push(value);
    }

    (keys, values)
}

criterion_group!(benches, insert_worse_case_benchmark);
criterion_main!(benches);
