//! Performance benchmarks for ethrex_db
//!
//! Run with: cargo bench

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use rand::prelude::*;

use ethrex_db::data::{NibblePath, SlottedArray};
use ethrex_db::merkle::{MerkleTrie, keccak256};
use ethrex_db::store::{PagedDb, CommitOptions, PageType, StateTrie, StorageTrie, AccountData};
use ethrex_db::chain::Blockchain;

/// Generate random bytes
fn random_bytes(len: usize) -> Vec<u8> {
    let mut rng = rand::thread_rng();
    (0..len).map(|_| rng.gen()).collect()
}

/// Benchmark NibblePath operations
fn bench_nibble_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("NibblePath");

    // From bytes
    let data = random_bytes(32);
    group.bench_function("from_bytes_32", |b| {
        b.iter(|| NibblePath::from_bytes(black_box(&data)))
    });

    // Get nibble
    let path = NibblePath::from_bytes(&data);
    group.bench_function("get_nibble", |b| {
        b.iter(|| path.get(black_box(30)))
    });

    // Common prefix
    let path2 = NibblePath::from_bytes(&random_bytes(32));
    group.bench_function("common_prefix_len", |b| {
        b.iter(|| path.common_prefix_len(black_box(&path2)))
    });

    // Slice
    group.bench_function("slice_from", |b| {
        b.iter(|| path.slice_from(black_box(10)))
    });

    group.finish();
}

/// Benchmark SlottedArray operations
fn bench_slotted_array(c: &mut Criterion) {
    let mut group = c.benchmark_group("SlottedArray");

    // Insert
    for size in [10, 100, 500].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("insert", size), size, |b, &size| {
            b.iter(|| {
                let mut arr = SlottedArray::new();
                for i in 0..size {
                    let key = NibblePath::from_bytes(&(i as u32).to_be_bytes());
                    let value = [i as u8; 32];
                    arr.try_insert(&key, &value);
                }
                arr
            })
        });
    }

    // Lookup
    let mut arr = SlottedArray::new();
    for i in 0..100 {
        let key = NibblePath::from_bytes(&(i as u32).to_be_bytes());
        arr.try_insert(&key, &[i as u8; 32]);
    }
    let lookup_key = NibblePath::from_bytes(&50u32.to_be_bytes());

    group.bench_function("get_100_entries", |b| {
        b.iter(|| arr.get(black_box(&lookup_key)))
    });

    group.finish();
}

/// Benchmark MerkleTrie operations
fn bench_merkle_trie(c: &mut Criterion) {
    let mut group = c.benchmark_group("MerkleTrie");

    // Insert single
    group.bench_function("insert_single", |b| {
        let mut trie = MerkleTrie::new();
        let key = keccak256(b"test_key");
        let value = vec![42u8; 64];
        b.iter(|| {
            trie.insert(black_box(&key), black_box(value.clone()));
        })
    });

    // Insert batch
    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("insert_batch", size), size, |b, &size| {
            let keys: Vec<_> = (0..size).map(|i: usize| keccak256(&i.to_be_bytes())).collect();
            let values: Vec<_> = (0..size).map(|i: usize| vec![i as u8; 64]).collect();

            b.iter(|| {
                let mut trie = MerkleTrie::new();
                for (key, value) in keys.iter().zip(values.iter()) {
                    trie.insert(key, value.clone());
                }
                trie
            })
        });
    }

    // Root hash computation
    let mut trie = MerkleTrie::new();
    for i in 0u32..100 {
        let key = keccak256(&i.to_be_bytes());
        trie.insert(&key, vec![i as u8; 64]);
    }
    group.bench_function("root_hash_100", |b| {
        b.iter(|| trie.root_hash())
    });

    // Get
    let lookup_key = keccak256(&50u32.to_be_bytes());
    group.bench_function("get_from_100", |b| {
        b.iter(|| trie.get(black_box(&lookup_key)))
    });

    group.finish();
}

/// Benchmark keccak256
fn bench_keccak(c: &mut Criterion) {
    let mut group = c.benchmark_group("Keccak256");

    for size in [32, 64, 128, 256, 1024].iter() {
        let data = random_bytes(*size);
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::new("hash", size), &data, |b, data| {
            b.iter(|| keccak256(black_box(data)))
        });
    }

    group.finish();
}

/// Benchmark StateTrie operations
fn bench_state_trie(c: &mut Criterion) {
    let mut group = c.benchmark_group("StateTrie");

    // Account insert
    group.bench_function("set_account", |b| {
        let mut state = StateTrie::new();
        let address = [42u8; 20];
        let account = AccountData::empty();

        b.iter(|| {
            state.set_account(black_box(&address), black_box(account.clone()));
        })
    });

    // Account batch insert
    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("set_accounts_batch", size), size, |b, &size| {
            let addresses: Vec<_> = (0..size)
                .map(|i| {
                    let mut addr = [0u8; 20];
                    addr[..4].copy_from_slice(&(i as u32).to_be_bytes());
                    addr
                })
                .collect();
            let account = AccountData::empty();

            b.iter(|| {
                let mut state = StateTrie::new();
                for addr in &addresses {
                    state.set_account(addr, account.clone());
                }
                state
            })
        });
    }

    // Root hash with accounts
    let mut state = StateTrie::new();
    for i in 0..100 {
        let mut addr = [0u8; 20];
        addr[..4].copy_from_slice(&(i as u32).to_be_bytes());
        state.set_account(&addr, AccountData::empty());
    }
    group.bench_function("root_hash_100_accounts", |b| {
        b.iter(|| state.root_hash())
    });

    group.finish();
}

/// Benchmark PagedDb operations
fn bench_paged_db(c: &mut Criterion) {
    let mut group = c.benchmark_group("PagedDb");

    // Create database
    group.bench_function("create_in_memory", |b| {
        b.iter(|| PagedDb::in_memory(black_box(100)).unwrap())
    });

    // Allocate pages
    group.bench_function("allocate_page", |b| {
        let mut db = PagedDb::in_memory(10000).unwrap();
        b.iter(|| {
            let mut batch = db.begin_batch();
            let result = batch.allocate_page(PageType::Data, 0);
            batch.abort();
            result
        })
    });

    // Commit batch
    group.bench_function("commit_batch_10_pages", |b| {
        let mut db = PagedDb::in_memory(10000).unwrap();
        b.iter(|| {
            let mut batch = db.begin_batch();
            for _ in 0..10 {
                batch.allocate_page(PageType::Data, 0).unwrap();
            }
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_nibble_path,
    bench_slotted_array,
    bench_merkle_trie,
    bench_keccak,
    bench_state_trie,
    bench_paged_db,
);
criterion_main!(benches);
