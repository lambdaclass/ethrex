use criterion::{criterion_group, criterion_main, Criterion};
use ethrex_common::{types::EMPTY_TRIE_HASH, H256};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use std::fs;

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
criterion_group!(benches, insert_many);
criterion_main!(benches);
