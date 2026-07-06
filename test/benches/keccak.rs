//! Microbenchmarks for the native keccak256 backend.
//!
//! Establishes the scalar single-lane baseline for the sizes ethrex actually
//! hashes: single words (`KECCAK256` opcode, storage-slot keys), trie-node
//! encodings, and larger blobs (tx/receipt/code). The `batch_baseline` group
//! measures many independent small hashes in a loop — the exact workload a
//! future 4-way AVX2 / FEAT_SHA3 kernel is meant to beat (see issue #6947).

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use ethrex_crypto::keccak::{Keccak256, keccak_hash, keccak256_batch};

/// Deterministic pseudo-random bytes (splitmix64) so runs are comparable.
fn pseudo_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut x = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    while out.len() < len {
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        out.extend_from_slice(&z.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn bench_one_shot(c: &mut Criterion) {
    // 136 = one keccak block (rate). 137 forces a second absorb call.
    let sizes = [32usize, 64, 128, 136, 137, 256, 512, 1024, 4096];
    let mut group = c.benchmark_group("keccak256/one_shot");
    for size in sizes {
        let input = pseudo_bytes(size, size as u64);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &input, |b, input| {
            b.iter(|| keccak_hash(black_box(input.as_slice())))
        });
    }
    group.finish();
}

fn bench_stream(c: &mut Criterion) {
    // Exercises the partial-block / tail-buffer path by feeding data in chunks.
    let size = 4096usize;
    let input = pseudo_bytes(size, 7);
    let mut group = c.benchmark_group("keccak256/stream");
    group.throughput(Throughput::Bytes(size as u64));
    for chunk in [1usize, 33, 136, 512] {
        group.bench_with_input(BenchmarkId::from_parameter(chunk), &chunk, |b, &chunk| {
            b.iter(|| {
                let mut hasher = Keccak256::new();
                for part in input.chunks(chunk) {
                    let _ = hasher.update(black_box(part));
                }
                hasher.finalize()
            })
        });
    }
    group.finish();
}

fn bench_batch_baseline(c: &mut Criterion) {
    // Scalar baseline for hashing many independent small inputs — the shape of
    // trie-node root computation and the target for SIMD batching (#6947).
    const N: usize = 1024;
    let mut group = c.benchmark_group("keccak256/batch_baseline");
    for size in [32usize, 64, 136] {
        let inputs: Vec<Vec<u8>> = (0..N)
            .map(|i| pseudo_bytes(size, (i as u64) << 8 | size as u64))
            .collect();
        group.throughput(Throughput::Elements(N as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &inputs, |b, inputs| {
            b.iter(|| {
                let mut acc = [0u8; 32];
                for input in inputs {
                    let h = keccak_hash(black_box(input.as_slice()));
                    for (a, x) in acc.iter_mut().zip(h) {
                        *a ^= x;
                    }
                }
                acc
            })
        });
    }
    group.finish();
}

fn bench_batch_avx2(c: &mut Criterion) {
    // 4-way batched keccak over the same workload as `batch_baseline`. Compare
    // the two groups at matching sizes to read the SIMD speedup (#6947).
    const N: usize = 1024;
    let mut group = c.benchmark_group("keccak256/batch_avx2");
    for size in [32usize, 64, 136] {
        let inputs: Vec<Vec<u8>> = (0..N)
            .map(|i| pseudo_bytes(size, (i as u64) << 8 | size as u64))
            .collect();
        let refs: Vec<&[u8]> = inputs.iter().map(|v| v.as_slice()).collect();
        group.throughput(Throughput::Elements(N as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &refs, |b, refs| {
            b.iter(|| keccak256_batch(black_box(refs)))
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_one_shot,
    bench_stream,
    bench_batch_baseline,
    bench_batch_avx2
);
criterion_main!(benches);
