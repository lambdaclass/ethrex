//! Performance benchmarks for ethrex_db
//!
//! Run with: cargo bench

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::prelude::*;

use ethrex_db::chain::Blockchain;
use ethrex_db::data::{NibblePath, SlottedArray};
use ethrex_db::merkle::{keccak256, MerkleTrie};
use ethrex_db::store::{AccountData, CommitOptions, PageType, PagedDb, StateTrie, StorageTrie};
use primitive_types::{H160, H256, U256};

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
    group.bench_function("get_nibble", |b| b.iter(|| path.get(black_box(30))));

    // Common prefix
    let path2 = NibblePath::from_bytes(&random_bytes(32));
    group.bench_function("common_prefix_len", |b| {
        b.iter(|| path.common_prefix_len(black_box(&path2)))
    });

    // Slice
    group.bench_function("slice_from", |b| b.iter(|| path.slice_from(black_box(10))));

    group.finish();
}

/// Non-SIMD prefix comparison for baseline measurement
fn scalar_starts_with(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    haystack[..needle.len()] == needle[..]
}

/// SIMD prefix comparison (same as in slotted_array.rs)
fn simd_starts_with(haystack: &[u8], needle: &[u8]) -> bool {
    use wide::u8x16;

    if needle.len() > haystack.len() {
        return false;
    }

    let needle_len = needle.len();
    let mut offset = 0;

    while offset + 16 <= needle_len {
        let h_chunk: [u8; 16] = haystack[offset..offset + 16].try_into().unwrap();
        let n_chunk: [u8; 16] = needle[offset..offset + 16].try_into().unwrap();

        let h_vec = u8x16::from(h_chunk);
        let n_vec = u8x16::from(n_chunk);

        let cmp = h_vec.cmp_eq(n_vec);
        if cmp.to_array().iter().any(|&b| b != 0xFF) {
            return false;
        }

        offset += 16;
    }

    haystack[offset..needle_len] == needle[offset..]
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

/// Benchmark SIMD vs scalar prefix comparison
fn bench_simd_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("SIMD_Comparison");

    // Test with different key sizes
    for key_size in [4, 16, 32, 64, 128].iter() {
        let haystack: Vec<u8> = (0..*key_size as u8).collect();
        let needle: Vec<u8> = (0..*key_size as u8).collect();

        group.bench_with_input(
            BenchmarkId::new("scalar", key_size),
            &(haystack.clone(), needle.clone()),
            |b, (h, n)| b.iter(|| scalar_starts_with(black_box(h), black_box(n))),
        );

        group.bench_with_input(
            BenchmarkId::new("simd", key_size),
            &(haystack, needle),
            |b, (h, n)| b.iter(|| simd_starts_with(black_box(h), black_box(n))),
        );
    }

    // Benchmark with worst case - mismatch at the end
    let haystack: Vec<u8> = (0..64u8).collect();
    let mut needle: Vec<u8> = (0..64u8).collect();
    needle[63] = 255; // Mismatch at the last byte

    group.bench_function("scalar_mismatch_end_64", |b| {
        b.iter(|| scalar_starts_with(black_box(&haystack), black_box(&needle)))
    });

    group.bench_function("simd_mismatch_end_64", |b| {
        b.iter(|| simd_starts_with(black_box(&haystack), black_box(&needle)))
    });

    group.finish();
}

/// Benchmark SlottedArray lookup with different key sizes
fn bench_slotted_array_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("SlottedArray_Lookup");

    // Create arrays with different key sizes
    for key_size in [4, 16, 32].iter() {
        // Fill array with 100 entries
        let mut arr = SlottedArray::new();
        for i in 0..100u32 {
            let key_bytes: Vec<u8> = (0..*key_size)
                .map(|j| ((i as u8).wrapping_add(j)))
                .collect();
            let key = NibblePath::from_bytes(&key_bytes);
            let value = [i as u8; 32];
            arr.try_insert(&key, &value);
        }

        // Lookup middle element
        let lookup_bytes: Vec<u8> = (0..*key_size).map(|j| (50u8.wrapping_add(j))).collect();
        let lookup_key = NibblePath::from_bytes(&lookup_bytes);

        group.bench_with_input(
            BenchmarkId::new("get_key_size", key_size),
            &(arr, lookup_key),
            |b, (arr, key)| b.iter(|| arr.get(black_box(key))),
        );
    }

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
            let keys: Vec<_> = (0..size)
                .map(|i: usize| keccak256(&i.to_be_bytes()))
                .collect();
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
    group.bench_function("root_hash_100", |b| b.iter(|| trie.root_hash()));

    // Get
    let lookup_key = keccak256(&50u32.to_be_bytes());
    group.bench_function("get_from_100", |b| {
        b.iter(|| trie.get(black_box(&lookup_key)))
    });

    group.finish();
}

/// Benchmark parallel vs sequential Merkle computation
fn bench_parallel_merkle(c: &mut Criterion) {
    let mut group = c.benchmark_group("Parallel_Merkle");

    for size in [100, 1000, 10000].iter() {
        // Create a trie with many entries
        let keys: Vec<_> = (0..*size)
            .map(|i: u32| keccak256(&i.to_be_bytes()))
            .collect();
        let values: Vec<_> = (0..*size).map(|i: u32| vec![i as u8; 64]).collect();

        // Sequential root hash
        group.bench_with_input(BenchmarkId::new("sequential", size), size, |b, _| {
            let mut trie = MerkleTrie::new();
            for (key, value) in keys.iter().zip(values.iter()) {
                trie.insert(key, value.clone());
            }
            b.iter(|| {
                trie.clear_cache();
                trie.root_hash()
            })
        });

        // Parallel root hash
        group.bench_with_input(BenchmarkId::new("parallel", size), size, |b, _| {
            let mut trie = MerkleTrie::new();
            for (key, value) in keys.iter().zip(values.iter()) {
                trie.insert(key, value.clone());
            }
            b.iter(|| {
                trie.clear_cache();
                trie.parallel_root_hash()
            })
        });
    }

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
        group.bench_with_input(
            BenchmarkId::new("set_accounts_batch", size),
            size,
            |b, &size| {
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
            },
        );
    }

    // Root hash with accounts
    let mut state = StateTrie::new();
    for i in 0..100 {
        let mut addr = [0u8; 20];
        addr[..4].copy_from_slice(&(i as u32).to_be_bytes());
        state.set_account(&addr, AccountData::empty());
    }
    group.bench_function("root_hash_100_accounts", |b| b.iter(|| state.root_hash()));

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
        b.iter(|| {
            let mut db = PagedDb::in_memory(100).unwrap();
            let mut batch = db.begin_batch();
            for _ in 0..10 {
                batch.allocate_page(PageType::Data, 0).unwrap();
            }
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
        })
    });

    group.finish();
}

/// Benchmark Bloom filter performance for non-existence lookups
fn bench_bloom_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("BloomFilter");

    // Create a trie with 10000 entries
    let mut trie = MerkleTrie::new();
    for i in 0u64..10000 {
        let key = keccak256(&i.to_be_bytes());
        trie.insert(&key, vec![i as u8; 32]);
    }

    // Benchmark: Look up keys that exist (Bloom filter passes, HashMap hit)
    let existing_key = keccak256(&5000u64.to_be_bytes());
    group.bench_function("lookup_existing_key", |b| {
        b.iter(|| trie.get(black_box(&existing_key)))
    });

    // Benchmark: Look up keys that don't exist (Bloom filter early rejection)
    // These keys are in a different range, so Bloom filter should reject them
    let nonexistent_key = keccak256(&99999u64.to_be_bytes());
    group.bench_function("lookup_nonexistent_key", |b| {
        b.iter(|| trie.get(black_box(&nonexistent_key)))
    });

    // Benchmark: may_contain check (raw Bloom filter)
    group.bench_function("may_contain_existing", |b| {
        b.iter(|| trie.may_contain(black_box(&existing_key)))
    });

    group.bench_function("may_contain_nonexistent", |b| {
        b.iter(|| trie.may_contain(black_box(&nonexistent_key)))
    });

    // Batch lookup of non-existent keys
    let nonexistent_keys: Vec<_> = (100000u64..100100)
        .map(|i| keccak256(&i.to_be_bytes()))
        .collect();
    group.throughput(Throughput::Elements(100));
    group.bench_function("batch_lookup_nonexistent_100", |b| {
        b.iter(|| {
            for key in &nonexistent_keys {
                black_box(trie.get(key));
            }
        })
    });

    group.finish();
}

/// Benchmark LRU page cache performance
fn bench_page_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("PageCache");

    // Create database with some pages
    let mut db = PagedDb::in_memory(1000).unwrap();
    let mut addrs = Vec::new();

    // Allocate 100 pages
    {
        let mut batch = db.begin_batch();
        for i in 0..100 {
            let (addr, _) = batch.allocate_page(PageType::Data, i as u8).unwrap();
            addrs.push(addr);
        }
        batch.commit(CommitOptions::DangerNoFlush).unwrap();
    }

    // Benchmark: First read (cache miss)
    // Clear cache first
    db.clear_cache();
    let addr = addrs[50];
    group.bench_function("first_read_cache_miss", |b| {
        b.iter(|| {
            db.clear_cache();
            db.get_page(black_box(addr))
        })
    });

    // Benchmark: Repeated read (cache hit)
    // Warm up cache
    let _ = db.get_page(addr);
    group.bench_function("repeated_read_cache_hit", |b| {
        b.iter(|| db.get_page(black_box(addr)))
    });

    // Report cache stats
    let (hits, misses) = db.cache_stats();
    println!(
        "Cache hits: {}, misses: {}, hit rate: {:.1}%",
        hits,
        misses,
        db.cache_hit_rate()
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_nibble_path,
    bench_slotted_array,
    bench_simd_comparison,
    bench_slotted_array_lookup,
    bench_merkle_trie,
    bench_parallel_merkle,
    bench_keccak,
    bench_state_trie,
    bench_paged_db,
    bench_bloom_filter,
    bench_page_cache,
);
criterion_main!(benches);
