use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

use ethrex_crypto::keccak::keccak_hash;

fn from_elem(c: &mut Criterion) {
    static KB: usize = 1024;

    let mut group = c.benchmark_group("from_elem");
    for size in [0, 1, 32, 64, 128, KB, 2 * KB, 4 * KB, 8 * KB, 16 * KB].iter() {
        let input: Vec<u8> = (0..*size).map(|i| (i % 256) as u8).collect();
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &input, |b, input| {
            b.iter(|| black_box(keccak_hash(black_box(input))));
        });
    }
    group.finish();
}

criterion_group!(benches, from_elem);
criterion_main!(benches);
