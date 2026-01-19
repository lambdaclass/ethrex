use criterion::{Criterion, criterion_group, criterion_main};

fn bench_encode_scalars(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_scalars");
    group.finish();
}

fn bench_encode_bytes_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_bytes_strings");
    group.finish();
}

fn bench_encode_collections(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_collections");
    group.finish();
}

fn bench_encode_tuples(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_tuples");
    group.finish();
}

fn bench_encode_ips(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_ip_types");
    group.finish();
}

criterion_group!(
    benches,
    bench_encode_scalars,
    bench_encode_bytes_strings,
    bench_encode_collections,
    bench_encode_tuples,
    bench_encode_ips,
);
criterion_main!(benches);
