//! Keccak-256 over the message sizes the EVM hashes most (one word, one rate block and its
//! edges) and a few multi-block sizes, through the backend the running CPU selects.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ethrex_crypto::keccak::keccak_hash;
use std::hint::black_box;

const SIZES: [usize; 10] = [0, 32, 64, 135, 136, 137, 256, 1024, 4096, 32 * 1024];

fn keccak(c: &mut Criterion) {
    let data: Vec<u8> = (0..32 * 1024).map(|i| (i * 31 + 7) as u8).collect();
    let mut group = c.benchmark_group("keccak_hash");
    for size in SIZES {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter(|| keccak_hash(black_box(&data[..size])))
        });
    }
    group.finish();
}

criterion_group!(benches, keccak);
criterion_main!(benches);
