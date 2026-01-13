use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ethrex_trie::Trie;
use uuid::Uuid;

fn insert_worse_case_benchmark(c: &mut Criterion) {
    c.bench_function("ethrex-trie insert one", |b| {
        let mut trie = Trie::new_temp();
        b.iter(|| {
            let key = Uuid::new_v4().as_bytes().to_vec();
            let value = Uuid::new_v4().as_bytes().to_vec();
            trie.insert(key, value).unwrap();
        })
    });

    c.bench_function("ethrex-trie insert 1k", |b| {
        let mut trie = Trie::new_temp();
        let (keys, values) = random_data(1000);
        b.iter(|| {
            for i in 0..keys.len() {
                trie.insert(keys[i].clone(), values[i].clone()).unwrap();
            }
        });
    });

    c.bench_function("ethrex-trie insert 10k", |b| {
        let mut trie = Trie::new_temp();
        let (keys, values) = random_data(10000);
        b.iter(|| {
            for i in 0..keys.len() {
                trie.insert(keys[i].clone(), values[i].clone()).unwrap();
            }
        });
    });
}

fn proof_benchmark(c: &mut Criterion) {
    let (trie_1k, keys_1k) = build_trie(1000);
    c.bench_function("ethrex-trie proof 1k", |b| {
        let mut i = 0usize;
        b.iter(|| {
            let key = &keys_1k[i % keys_1k.len()];
            i += 1;
            black_box(trie_1k.get_proof(key).unwrap());
        })
    });

    let (trie_10k, keys_10k) = build_trie(10000);
    c.bench_function("ethrex-trie proof 10k", |b| {
        let mut i = 0usize;
        b.iter(|| {
            let key = &keys_10k[i % keys_10k.len()];
            i += 1;
            black_box(trie_10k.get_proof(key).unwrap());
        })
    });
}

fn hash_benchmark(c: &mut Criterion) {
    c.bench_function("ethrex-trie hash 1k", |b| {
        let (keys, values) = random_data(1000);
        let mut trie = Trie::new_temp();
        for i in 0..keys.len() {
            trie.insert(keys[i].clone(), values[i].clone()).unwrap();
        }
        let (_, update_values) = random_data(1000);
        let mut i = 0usize;
        b.iter(|| {
            let key = &keys[i % keys.len()];
            let value = &update_values[i % update_values.len()];
            trie.insert(key.clone(), value.clone()).unwrap();
            black_box(trie.hash().unwrap());
            i += 1;
        })
    });

    c.bench_function("ethrex-trie hash 10k", |b| {
        let (keys, values) = random_data(10000);
        let mut trie = Trie::new_temp();
        for i in 0..keys.len() {
            trie.insert(keys[i].clone(), values[i].clone()).unwrap();
        }
        let (_, update_values) = random_data(10000);
        let mut i = 0usize;
        b.iter(|| {
            let key = &keys[i % keys.len()];
            let value = &update_values[i % update_values.len()];
            trie.insert(key.clone(), value.clone()).unwrap();
            black_box(trie.hash().unwrap());
            i += 1;
        })
    });
}

fn build_trie(n: usize) -> (Trie, Vec<Vec<u8>>) {
    let (keys, values) = random_data(n);
    let mut trie = Trie::new_temp();
    for i in 0..keys.len() {
        trie.insert(keys[i].clone(), values[i].clone()).unwrap();
    }
    (trie, keys)
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

criterion_group!(
    benches,
    insert_worse_case_benchmark,
    proof_benchmark,
    hash_benchmark
);
criterion_main!(benches);
