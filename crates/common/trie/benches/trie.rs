use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use criterion::{criterion_group, criterion_main, Criterion};

use hasher::HasherKeccak;
use uuid::Uuid;

use cita_trie::{MemoryDB as CitaMemoryDB, PatriciaTrie as CitaPatriciaTrie, Trie as CitaTrie};

use ethrex_trie::{InMemoryTrieDB as EthrexMemoryDB, Trie as EthrexTrie};

fn cita_new_trie() -> CitaPatriciaTrie<CitaMemoryDB, HasherKeccak> {
    CitaPatriciaTrie::new(
        Arc::new(CitaMemoryDB::new(false)),
        Arc::new(HasherKeccak::new()),
    )
}

fn ethrex_new_trie() -> EthrexTrie {
    let hmap: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let map = Arc::new(Mutex::new(hmap));
    let db = EthrexMemoryDB::new(map);
    EthrexTrie::new(Box::new(db))
}

fn cita_trie_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("CitaTrie");

    group.bench_function("insert one", |b| {
        let mut trie = cita_new_trie();

        b.iter(|| {
            let key = Uuid::new_v4().as_bytes().to_vec();
            let value = Uuid::new_v4().as_bytes().to_vec();
            trie.insert(key, value).unwrap()
        })
    });

    group.bench_function("insert 1k", |b| {
        let mut trie = cita_new_trie();

        let (keys, values) = random_data(1000);
        b.iter(|| {
            for i in 0..keys.len() {
                trie.insert(keys[i].clone(), values[i].clone()).unwrap()
            }
        });
    });

    group.bench_function("insert 10k", |b| {
        let mut trie = cita_new_trie();

        let (keys, values) = random_data(10000);
        b.iter(|| {
            for i in 0..keys.len() {
                trie.insert(keys[i].clone(), values[i].clone()).unwrap()
            }
        });
    });

    group.bench_function("get based 10k", |b| {
        let mut trie = cita_new_trie();

        let (keys, values) = random_data(10000);
        for i in 0..keys.len() {
            trie.insert(keys[i].clone(), values[i].clone()).unwrap()
        }

        b.iter(|| {
            let key = trie.get(&keys[7777]).unwrap();
            assert_ne!(key, None);
        });
    });

    group.bench_function("remove 1k", |b| {
        let mut trie = cita_new_trie();

        let (keys, values) = random_data(1000);
        for i in 0..keys.len() {
            trie.insert(keys[i].clone(), values[i].clone()).unwrap()
        }

        b.iter(|| {
            for key in keys.iter() {
                trie.remove(key).unwrap();
            }
        });
    });

    group.bench_function("remove 10k", |b| {
        let mut trie = cita_new_trie();

        let (keys, values) = random_data(10000);
        for i in 0..keys.len() {
            trie.insert(keys[i].clone(), values[i].clone()).unwrap()
        }

        b.iter(|| {
            for key in keys.iter() {
                trie.remove(key).unwrap();
            }
        });
    });

    group.finish();
}


fn ethrex_trie_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("EthrexTrie");

    group.bench_function("insert one", |b| {
        let mut trie = ethrex_new_trie();

        b.iter(|| {
            let key = Uuid::new_v4().as_bytes().to_vec();
            let value = Uuid::new_v4().as_bytes().to_vec();
            trie.insert(key, value).unwrap()
        })
    });

    group.bench_function("insert 1k", |b| {
        let mut trie = ethrex_new_trie();

        let (keys, values) = random_data(1000);
        b.iter(|| {
            for i in 0..keys.len() {
                trie.insert(keys[i].clone(), values[i].clone()).unwrap()
            }
        });
    });

    group.bench_function("insert 10k", |b| {
        let mut trie = ethrex_new_trie();

        let (keys, values) = random_data(10000);
        b.iter(|| {
            for i in 0..keys.len() {
                trie.insert(keys[i].clone(), values[i].clone()).unwrap()
            }
        });
    });

    group.bench_function("get based 10k", |b| {
        let mut trie = ethrex_new_trie();

        let (keys, values) = random_data(10000);
        for i in 0..keys.len() {
            trie.insert(keys[i].clone(), values[i].clone()).unwrap()
        }

        b.iter(|| {
            let key = trie.get(&keys[7777]).unwrap();
            assert_ne!(key, None);
        });
    });

    group.bench_function("remove 1k", |b| {
        let mut trie = ethrex_new_trie();

        let (keys, values) = random_data(1000);
        for i in 0..keys.len() {
            trie.insert(keys[i].clone(), values[i].clone()).unwrap()
        }

        b.iter(|| {
            for key in keys.iter() {
                trie.remove(key).unwrap();
            }
        });
    });

    group.bench_function("remove 10k", |b| {
        let mut trie = ethrex_new_trie();

        let (keys, values) = random_data(10000);
        for i in 0..keys.len() {
            trie.insert(keys[i].clone(), values[i].clone()).unwrap()
        }

        b.iter(|| {
            for key in keys.iter() {
                trie.remove(key).unwrap();
            }
        });
    });

    group.finish();
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

criterion_group!(benches, cita_trie_benchmark, ethrex_trie_benchmark);
criterion_main!(benches);
