use criterion::{BatchSize, Bencher, BenchmarkId, Criterion, criterion_group, criterion_main};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use rand::{
    Rng,
    distr::{Alphanumeric, Distribution, SampleString, StandardUniform},
};
use std::{hint::black_box, iter::repeat_with};

fn bench_decode_scalars(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_scalars");

    fn impl_bench_for<T, const N: usize>(b: &mut Bencher)
    where
        StandardUniform: Distribution<T>,
        T: RLPDecode + RLPEncode,
    {
        b.iter_batched_ref(
            || {
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

fn bench_decode_lists(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_lists");

    fn impl_bench_for<T, const N: usize, const L: usize>(b: &mut Bencher)
    where
        StandardUniform: Distribution<T>,
        T: RLPDecode + RLPEncode,
    {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        rand::rng()
                            .random_iter::<T>()
                            .take(L)
                            .collect::<Vec<_>>()
                            .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Vec::<T>::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    fn impl_bench_str<const N: usize, const L: usize>(b: &mut Bencher) {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        repeat_with(|| Alphanumeric.sample_string(&mut rand::rng(), 10))
                            .take(L)
                            .collect::<Vec<_>>()
                            .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Vec::<String>::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(
        BenchmarkId::new("u8", "len=10/1000"),
        impl_bench_for::<u8, 1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("u8", "len=100/1000"),
        impl_bench_for::<u8, 1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("u8", "len=1000/1000"),
        impl_bench_for::<u8, 1000, 1000>,
    );

    group.bench_function(
        BenchmarkId::new("u16", "len=10/1000"),
        impl_bench_for::<u16, 1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("u16", "len=100/1000"),
        impl_bench_for::<u16, 1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("u16", "len=1000/1000"),
        impl_bench_for::<u16, 1000, 1000>,
    );

    group.bench_function(
        BenchmarkId::new("u32", "len=10/1000"),
        impl_bench_for::<u32, 1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("u32", "len=100/1000"),
        impl_bench_for::<u32, 1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("u32", "len=1000/1000"),
        impl_bench_for::<u32, 1000, 1000>,
    );

    group.bench_function(
        BenchmarkId::new("u64", "len=10/1000"),
        impl_bench_for::<u64, 1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("u64", "len=100/1000"),
        impl_bench_for::<u64, 1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("u64", "len=1000/1000"),
        impl_bench_for::<u64, 1000, 1000>,
    );

    // group.bench_function("u128", impl_bench_for::<u128>);

    group.bench_function(
        BenchmarkId::new("str[10]", "len=10/1000"),
        impl_bench_str::<1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("str[10]", "len=100/1000"),
        impl_bench_str::<1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("str[10]", "len=1000/1000"),
        impl_bench_str::<1000, 1000>,
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_decode_scalars,
    bench_decode_bytes,
    bench_decode_strings,
    bench_decode_lists,
);
criterion_main!(benches);
