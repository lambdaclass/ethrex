use cita_trie::{MemoryDB, Trie};
use criterion::{criterion_group, criterion_main, Criterion};
use ethrex_core::{types::EMPTY_TRIE_HASH, H256};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use hasher::HasherKeccak;
use std::{fs, sync::Arc};

fn insert_many(c: &mut Criterion) {
    const BENCH_DB_DIR: &str = "bench_db";
    let store = Store::new(BENCH_DB_DIR, ethrex_storage::EngineType::Libmdbx).unwrap();
    let mut trie = store.open_state_trie(*EMPTY_TRIE_HASH);
    c.bench_function("insert", |b| {
        b.iter(|| {
            for _ in 0..100 {
                trie.insert(
                    H256::random().encode_to_vec(),
                    H256::random().encode_to_vec(),
                )
                .unwrap();
            }
            trie.hash().unwrap()
        })
    });
    fs::remove_dir_all(BENCH_DB_DIR).expect("Failed to clean bench db dir");
}

fn insert_many_in_memory(c: &mut Criterion) {
    let mut trie = ethrex_trie::Trie::new_in_memory();
    c.bench_function("insert_many_in_memory", |b| {
        b.iter(|| {
            for _ in 0..100 {
                trie.insert(
                    H256::random().encode_to_vec(),
                    H256::random().encode_to_vec(),
                )
                .unwrap();
            }
            trie.hash().unwrap()
        })
    });
}

fn insert_many_cita_in_memory(c: &mut Criterion) {
    let db = Arc::new(MemoryDB::new(true));
    let hasher = Arc::new(HasherKeccak::new());

    let mut trie = cita_trie::PatriciaTrie::new(Arc::clone(&db), Arc::clone(&hasher));
    // const BENCH_DB_DIR: &str = "bench_db";
    // let store = Store::new(BENCH_DB_DIR, ethrex_storage::EngineType::Libmdbx).unwrap();
    // let mut trie = store.open_state_trie(*EMPTY_TRIE_HASH);
    c.bench_function("insert_many_in_memory_cita", |b| {
        b.iter(|| {
            for _ in 0..100 {
                trie.insert(
                    H256::random().encode_to_vec(),
                    H256::random().encode_to_vec(),
                )
                .unwrap();
            }
            trie.root().unwrap()
        })
    });
}
criterion_group!(
    benches,
    insert_many,
    insert_many_in_memory,
    insert_many_cita_in_memory
);
criterion_main!(benches);
