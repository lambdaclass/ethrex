//! `DIV`, `MOD`, `ADDMOD` and `MULMOD` arithmetic on the worst-case operand shapes of the
//! compute benchmarks, against the generic long division of the `uint` crate.
//!
//! Like the benchmark contracts, each `MOD`-style chain feeds every result back as the
//! next modulus, picking from a fixed pool the numerator that keeps the modulus largest,
//! so the modulus stays near its starting width for hundreds of steps and the timing is
//! latency-bound. `DIV` alternates between dividing by `2^128 + 0x33` and by the
//! previous quotient, the worst case for one- and two-word fast paths.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ethereum_types::{U256, U512};
use ethrex_crypto::u256_div;
use std::hint::black_box;

fn uint_div(a: U256, b: U256) -> U256 {
    a.checked_div(b).unwrap_or_default()
}

fn uint_rem(a: U256, b: U256) -> U256 {
    a.checked_rem(b).unwrap_or_default()
}

fn uint_add_mod(a: U256, b: U256, m: U256) -> U256 {
    if m.is_zero() {
        return U256::zero();
    }
    let r = (U512::from(a) + U512::from(b)) % U512::from(m);
    U256([r.0[0], r.0[1], r.0[2], r.0[3]])
}

fn uint_mul_mod(a: U256, b: U256, m: U256) -> U256 {
    if m.is_zero() {
        return U256::zero();
    }
    let r = a.full_mul(b) % U512::from(m);
    U256([r.0[0], r.0[1], r.0[2], r.0[3]])
}

/// SplitMix64, for reproducible operands.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A random word with exactly `bits` significant bits.
fn word(state: &mut u64, bits: usize) -> U256 {
    let x = U256([next(state), next(state), next(state), next(state)]);
    let x = if bits < 256 {
        x & ((U256::one() << bits) - 1)
    } else {
        x
    };
    x | (U256::one() << (bits - 1))
}

/// Operand pool, starting modulus and the greedy order of numerators that keeps the
/// modulus above `2^(mod_bits - 63)` for as long as possible.
struct Chain {
    pool: Vec<U256>,
    initial: U256,
    order: Vec<usize>,
}

fn chain(mod_bits: usize, step: impl Fn(U256, U256) -> U256) -> Chain {
    let mut state = mod_bits as u64;
    let pool: Vec<U256> = (0..15).map(|_| word(&mut state, 256)).collect();
    let initial = word(&mut state, mod_bits);
    let floor = U256::one() << (mod_bits - 63);
    let (mut modulus, mut order) = (initial, Vec::new());
    while modulus >= floor && order.len() < 1000 {
        let (i, next) = pool
            .iter()
            .map(|&x| step(x, modulus))
            .enumerate()
            .max_by_key(|&(_, r)| r)
            .unwrap();
        order.push(i);
        modulus = next;
    }
    Chain {
        pool,
        initial,
        order,
    }
}

fn run_chain(c: &Chain, step: impl Fn(U256, U256) -> U256) -> U256 {
    let mut modulus = black_box(c.initial);
    for &i in &c.order {
        modulus = step(c.pool[i], modulus);
    }
    modulus
}

fn bench(c: &mut Criterion) {
    let numerator = U256::from_str_radix(
        "fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f",
        16,
    )
    .unwrap();
    let divisor = (U256::one() << 128) + 0x33;
    let mut group = c.benchmark_group("DIV 2^256 by 2^128 + 0x33 (1000 ops)");
    for (name, f) in [
        ("uint", uint_div as fn(U256, U256) -> U256),
        ("u256_div", u256_div::div),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| {
                let mut v = black_box(divisor);
                for _ in 0..1000 {
                    v = f(numerator, v);
                }
                v
            })
        });
    }
    group.finish();

    type Step = fn(U256, U256) -> U256;
    let ops: [(&str, Step, Step); 3] = [
        ("MOD", uint_rem, u256_div::rem),
        (
            "ADDMOD",
            |x, m| uint_add_mod(x, U256::MAX, m),
            |x, m| u256_div::add_mod(x, U256::MAX, m),
        ),
        (
            "MULMOD",
            |x, m| uint_mul_mod(x, U256::MAX, m),
            |x, m| u256_div::mul_mod(x, U256::MAX, m),
        ),
    ];
    for mod_bits in [63, 127, 191, 255] {
        for (op, uint_step, step) in ops {
            let chain = chain(mod_bits, step);
            let mut group = c.benchmark_group(format!(
                "{op} chain, {mod_bits}-bit modulus ({} ops)",
                chain.order.len()
            ));
            for (name, f) in [("uint", uint_step), ("u256_div", step)] {
                group.bench_function(BenchmarkId::from_parameter(name), |b| {
                    b.iter(|| run_chain(&chain, f))
                });
            }
            group.finish();
        }
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
