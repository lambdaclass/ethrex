//! Benchmarks for Merkle Patricia Trie construction and root computation.
//!
//! Root computation is keccak-bound: every node is RLP-encoded and hashed, and
//! siblings are hash-independent. These benches track that cost across trie
//! sizes and cover both the incremental `insert`+`hash_no_commit` path and the
//! parallel sorted-merkleization path used for state roots (see #6947).

use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use ethereum_types::H256;
use ethrex_crypto::NativeCrypto;
use ethrex_trie::Trie;
use ethrex_trie::trie_sorted::trie_from_sorted_accounts_wrap;

/// Deterministic pseudo-random 32-byte trie key (splitmix64).
fn key(seed: u64) -> H256 {
    let mut bytes = [0u8; 32];
    let mut x = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    for chunk in bytes.chunks_mut(8) {
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        chunk.copy_from_slice(&z.to_le_bytes()[..chunk.len()]);
    }
    H256(bytes)
}

/// ~72-byte value, a proxy for an RLP-encoded account (nonce, balance,
/// storage_root, code_hash).
fn value(seed: u64) -> Vec<u8> {
    let mut v = vec![0u8; 72];
    v[..8].copy_from_slice(&seed.to_le_bytes());
    v
}

fn dataset(n: usize) -> Vec<(H256, Vec<u8>)> {
    (0..n as u64).map(|i| (key(i), value(i))).collect()
}

fn bench_insert_and_root(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie/insert_and_root");
    for n in [1_000usize, 10_000, 100_000] {
        let data = dataset(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &data, |b, data| {
            b.iter(|| {
                let mut trie = Trie::new_temp();
                for (k, v) in data {
                    trie.insert(k.as_bytes().to_vec(), v.clone()).unwrap();
                }
                black_box(trie.hash_no_commit(&NativeCrypto))
            })
        });
    }
    group.finish();
}

fn bench_root_only(c: &mut Criterion) {
    // Isolates root computation (RLP + keccak) from insertion: a fresh
    // fully-inserted, unhashed trie is built per iteration in untimed setup.
    let mut group = c.benchmark_group("trie/root_only");
    for n in [1_000usize, 10_000, 100_000] {
        let data = dataset(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &data, |b, data| {
            b.iter_batched(
                || {
                    let mut trie = Trie::new_temp();
                    for (k, v) in data {
                        trie.insert(k.as_bytes().to_vec(), v.clone()).unwrap();
                    }
                    trie
                },
                |trie| black_box(trie.hash_no_commit(&NativeCrypto)),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_sorted_merkleize(c: &mut Criterion) {
    // Parallel sorted-account merkleization — the state-root build path.
    let mut group = c.benchmark_group("trie/sorted_merkleize");
    for n in [1_000usize, 10_000, 100_000] {
        let mut data = dataset(n);
        data.sort_by(|a, b| a.0.cmp(&b.0));
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &data, |b, data| {
            b.iter_batched(
                || data.clone(),
                |data| {
                    let trie = Trie::new_temp();
                    let db = trie.db();
                    black_box(trie_from_sorted_accounts_wrap(db, &mut data.into_iter()).unwrap())
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_insert_and_root,
    bench_root_only,
    bench_sorted_merkleize
);
criterion_main!(benches);
