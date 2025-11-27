use criterion::{Criterion, criterion_group, criterion_main};

fn bench_decode_scalars(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_scalars");
    group.finish();
}

fn bench_decode_bytes_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bytes_strings");
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
    bench_decode_bytes_strings,
    bench_decode_collections,
    bench_decode_tuples,
    bench_decode_ips,
);
criterion_main!(benches);
