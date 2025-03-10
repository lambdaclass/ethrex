#![allow(unused)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::{thread, time::Duration};
#[inline]
fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 1,
        1 => 1,
        n => fibonacci(n - 1) + fibonacci(n - 2),
    }
}


pub fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("fib 20", |b| b.iter(|| fibonacci(black_box(20))));
}

criterion_group!(runner, criterion_benchmark);
criterion_main!(runner);
