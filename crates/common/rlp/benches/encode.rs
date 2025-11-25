use criterion::{Criterion, criterion_group, criterion_main};
use ethereum_types::U256;
use ethrex_common::{H256, types::AccountInfo};
use ethrex_rlp::encode::RLPEncode;
use std::hint::black_box;

fn make_string_list(count: usize) -> Vec<String> {
    let entry = "abcdefghij".to_string();
    vec![entry; count]
}

fn make_u256_with_len(len: usize) -> U256 {
    assert!((1..=32).contains(&len));
    let shift = len.saturating_mul(8).saturating_sub(1);
    U256::from(1u64) << shift
}

fn bench_encode_integer(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_integer");

    group.bench_function("u8", |b| {
        let mut buf = Vec::new();
        let value: u8 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u8::MAX", |b| {
        let mut buf = Vec::new();
        let value: u8 = u8::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u16", |b| {
        let mut buf = Vec::new();
        let value: u16 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u16::MAX", |b| {
        let mut buf = Vec::new();
        let value: u16 = u16::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u32", |b| {
        let mut buf = Vec::new();
        let value: u32 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u32 > 0x7f", |b| {
        let mut buf = Vec::new();
        let value: u32 = u32::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u64", |b| {
        let mut buf = Vec::new();
        let value: u64 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u64::MAX", |b| {
        let mut buf = Vec::new();
        let value: u64 = u64::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u128", |b| {
        let mut buf = Vec::new();
        let value: u128 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u128::MAX", |b| {
        let mut buf = Vec::new();
        let value: u128 = u128::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u256", |b| {
        let mut buf = Vec::new();
        let value: U256 = U256::from(42u64);
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u256::MAX", |b| {
        let mut buf = Vec::new();
        let value: U256 = U256::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_integer_lengths(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_integer_lengths");
    for &len in &[1usize, 2, 4, 8, 16, 32] {
        let label = format!("bytes_{len}");
        let value = make_u256_with_len(len);
        group.bench_function(label, move |b| {
            let mut buf = Vec::new();
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_strings");
    for &len in &[5usize, 60, 500] {
        let label = format!("len_{len}");
        let value = "a".repeat(len);
        group.bench_function(label, move |b| {
            let mut buf = Vec::new();
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_int_lists(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_int_lists");
    for &count in &[10usize, 100, 1000] {
        let label = format!("len_{count}");
        let value: Vec<_> = (0..count as u64).collect();
        group.bench_function(label, move |b| {
            let mut buf = Vec::new();
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_string_lists(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_string_lists");
    for &count in &[10usize, 100, 1000] {
        let label = format!("len_{count}");
        let value = make_string_list(count);
        group.bench_function(label, move |b| {
            let mut buf = Vec::new();
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_account_info(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_account_info");

    let account_info = AccountInfo {
        code_hash: H256::repeat_byte(0xab),
        balance: U256::from(0xf34ab23u64),
        nonce: 1,
    };

    group.bench_function("account_info", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            account_info.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_encode_integer,
    bench_encode_integer_lengths,
    bench_encode_strings,
    bench_encode_int_lists,
    bench_encode_string_lists,
    bench_encode_account_info
);
criterion_main!(benches);
