use criterion::{BatchSize, Bencher, BenchmarkId, Criterion, criterion_group, criterion_main};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use rand::{
    Rng,
    distr::{Alphanumeric, Distribution, SampleString, StandardUniform},
};
use std::hint::black_box;

fn bench_decode_scalars(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_scalars");

    fn impl_bench_for<T, const N: usize>(b: &mut Bencher)
    where
        StandardUniform: Distribution<T>,
        T: RLPDecode + RLPEncode,
    {
        b.iter_batched_ref(
            move || {
                rand::rng()
                    .random_iter::<T>()
                    .take(N)
                    .map(|x| x.encode_to_vec())
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data.iter() {
                    black_box(T::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(BenchmarkId::new("u8", 1000), impl_bench_for::<u8, 1000>);
    group.bench_function(BenchmarkId::new("u16", 1000), impl_bench_for::<u16, 1000>);
    group.bench_function(BenchmarkId::new("u32", 1000), impl_bench_for::<u32, 1000>);
    group.bench_function(BenchmarkId::new("u64", 1000), impl_bench_for::<u64, 1000>);
    // group.bench_function("u128", impl_bench_for::<u128>);

    group.finish();
}

fn bench_decode_bytes(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bytes");

    fn impl_bench<const N: usize, const L: usize>(b: &mut Bencher) {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        rand::rng()
                            .random_iter::<u8>()
                            .take(L)
                            .collect::<Vec<_>>()
                            .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Vec::<u8>::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(
        BenchmarkId::new("[u8]", "len=5/1000"),
        impl_bench::<1000, 5>,
    );
    group.bench_function(
        BenchmarkId::new("[u8]", "len=60/1000"),
        impl_bench::<1000, 60>,
    );
    group.bench_function(
        BenchmarkId::new("[u8]", "len=500/1000"),
        impl_bench::<1000, 500>,
    );

    group.finish();
}

fn bench_decode_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_strings");

    fn impl_bench<const N: usize, const L: usize>(b: &mut Bencher) {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        Alphanumeric
                            .sample_string(&mut rand::rng(), L)
                            .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(String::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(BenchmarkId::new("str", "len=5/1000"), impl_bench::<1000, 5>);
    group.bench_function(
        BenchmarkId::new("str", "len=60/1000"),
        impl_bench::<1000, 60>,
    );
    group.bench_function(
        BenchmarkId::new("str", "len=500/1000"),
        impl_bench::<1000, 500>,
    );

    group.finish();
}

fn bench_decode_collections(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_collections");
    group.finish();
}

fn bench_decode_tuples(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_tuples");
    group.finish();
}

fn bench_decode_ips(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_ip_types");
    group.finish();
}

criterion_group!(
    benches,
    bench_decode_scalars,
    bench_decode_bytes,
    bench_decode_collections,
    bench_decode_tuples,
    bench_decode_ips,
);
criterion_main!(benches);
