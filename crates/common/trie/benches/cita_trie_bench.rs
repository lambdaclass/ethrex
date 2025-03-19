// This code is originally from https://github.com/citahub/cita_trie/ and
// modified to suit our needs, and to have a baseline to benchmark our own
// trie implementation against an existing one.

use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};

use hasher::HasherKeccak;
use uuid::Uuid;

use cita_trie::MemoryDB;
use cita_trie::{PatriciaTrie, Trie};
use ethrex_trie::InMemoryTrieDB;
use ethrex_trie::Trie as EthrexTrie;
use ethrex_trie::TrieDB;

fn insert_worse_case_benchmark(c: &mut Criterion) {
    let key = Uuid::new_v4().as_bytes().to_vec();
    let value = Uuid::new_v4().as_bytes().to_vec();
    c.bench_function("cita-trie insert one", |b| {
        let mut trie = PatriciaTrie::new(
            Arc::new(MemoryDB::new(false)),
            Arc::new(HasherKeccak::new()),
        );

        b.iter(|| trie.insert(key.clone(), value.clone()).unwrap());
    });

    let (keys, values) = random_data(1000);

    c.bench_function("cita-trie insert 1k", |b| {
        let mut trie = PatriciaTrie::new(
            Arc::new(MemoryDB::new(false)),
            Arc::new(HasherKeccak::new()),
        );

        b.iter(|| {
            for i in 0..keys.len() {
                trie.insert(keys[i].clone(), values[i].clone()).unwrap()
            }
        });
    });

    let (keys, values) = random_data(10000);

    c.bench_function("cita-trie insert 10k", |b| {
        let mut trie = PatriciaTrie::new(
            Arc::new(MemoryDB::new(false)),
            Arc::new(HasherKeccak::new()),
        );

        b.iter(|| {
            for i in 0..keys.len() {
                trie.insert(keys[i].clone(), values[i].clone()).unwrap()
            }
        });
    });
}

fn random_data(n: usize) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut keys = Vec::with_capacity(n);
    let mut values = Vec::with_capacity(n);
    for _ in 0..n {
        let key = Uuid::new_v4().as_bytes().to_vec();
        let value = Uuid::new_v4().as_bytes().to_vec();
        keys.push(key);
        values.push(value);
    }

    (keys, values)
}

criterion_group!(benches, insert_worse_case_benchmark);
criterion_main!(benches);
