//! Microbenchmarks for the BN254 precompile primitives (ECADD, ECMUL and
//! ECPAIRING) through `NativeCrypto`.

use ark_ec::{AffineRepr, CurveGroup};
use ark_ff::{BigInteger, PrimeField};
use criterion::{Criterion, criterion_group, criterion_main};
use ethrex_crypto::{Crypto, NativeCrypto};
use std::hint::black_box;

fn g1_bytes(p: &ark_bn254::G1Affine) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&p.x.into_bigint().to_bytes_be());
    out[32..].copy_from_slice(&p.y.into_bigint().to_bytes_be());
    out
}

fn g2_bytes(p: &ark_bn254::G2Affine) -> [u8; 128] {
    let mut out = [0u8; 128];
    out[..32].copy_from_slice(&p.x.c1.into_bigint().to_bytes_be());
    out[32..64].copy_from_slice(&p.x.c0.into_bigint().to_bytes_be());
    out[64..96].copy_from_slice(&p.y.c1.into_bigint().to_bytes_be());
    out[96..].copy_from_slice(&p.y.c0.into_bigint().to_bytes_be());
    out
}

fn bench_bn254(c: &mut Criterion) {
    let g = ark_bn254::G1Affine::generator();
    let p1 = (g * ark_bn254::Fr::from(123_456_789u64)).into_affine();
    let p2 = (g * ark_bn254::Fr::from(987_654_321u64)).into_affine();
    let a = g1_bytes(&p1);
    let b = g1_bytes(&p2);
    let scalar = [0xffu8; 32];
    let crypto = NativeCrypto;

    c.bench_function("bn254_add", |bench| {
        bench.iter(|| crypto.bn254_g1_add(black_box(&a), black_box(&b)).unwrap())
    });
    c.bench_function("bn254_double", |bench| {
        bench.iter(|| crypto.bn254_g1_add(black_box(&a), black_box(&a)).unwrap())
    });
    // Scalars 2 and 2^bits - 1 (all bits set), below and around the GLV threshold.
    for bits in [2usize, 32, 64, 96, 128, 160, 192] {
        let mut small = [0u8; 32];
        if bits == 2 {
            small[31] = 2;
        } else {
            small[32 - bits / 8..].fill(0xff);
        }
        c.bench_function(&format!("bn254_mul_{bits}_bit_scalar"), |bench| {
            bench.iter(|| {
                crypto
                    .bn254_g1_mul(black_box(&a), black_box(&small))
                    .unwrap()
            })
        });
    }
    c.bench_function("bn254_mul_max_scalar", |bench| {
        bench.iter(|| {
            crypto
                .bn254_g1_mul(black_box(&a), black_box(&scalar))
                .unwrap()
        })
    });

    let h = ark_bn254::G2Affine::generator();
    let q = g2_bytes(&h);
    // e(P, Q) * e(-P, Q) == 1
    let neg = g1_bytes(&(-p1));
    let pairs: Vec<(&[u8], &[u8])> = vec![(&a, &q), (&neg, &q)];
    c.bench_function("bn254_pairing_2", |bench| {
        bench.iter(|| crypto.bn254_pairing_check(black_box(&pairs)).unwrap())
    });

    // Random pairs (k1 * G1, k2 * G2) whose product is not one, as in the
    // benchmark-suite inputs.
    let random: Vec<([u8; 64], [u8; 128])> = (0..24u64)
        .map(|i| {
            let k1 = ark_bn254::Fr::from(1_000_003 * (i + 1));
            let k2 = ark_bn254::Fr::from(2_000_029 * (i + 7));
            (
                g1_bytes(&(g * k1).into_affine()),
                g2_bytes(&(h * k2).into_affine()),
            )
        })
        .collect();
    for n in [1usize, 3, 6, 12, 24] {
        let pairs: Vec<(&[u8], &[u8])> = random[..n]
            .iter()
            .map(|(p, q)| (p.as_slice(), q.as_slice()))
            .collect();
        c.bench_function(&format!("bn254_pairing_random_{n}"), |bench| {
            bench.iter(|| crypto.bn254_pairing_check(black_box(&pairs)).unwrap())
        });
    }
}

fn bench_pairing_parts(c: &mut Criterion) {
    use ark_ec::pairing::Pairing;
    let g = ark_bn254::G1Affine::generator();
    let h = ark_bn254::G2Affine::generator();
    let p = (g * ark_bn254::Fr::from(7u64)).into_affine();
    let q = (h * ark_bn254::Fr::from(11u64)).into_affine();
    c.bench_function("part_g1_subgroup_oncurve", |b| {
        b.iter(|| black_box(&p).is_on_curve())
    });
    c.bench_function("part_g2_subgroup_check", |b| {
        b.iter(|| black_box(&q).is_in_correct_subgroup_assuming_on_curve())
    });
    c.bench_function("part_miller_loop_1", |b| {
        b.iter(|| ark_bn254::Bn254::multi_miller_loop([black_box(p)], [black_box(q)]))
    });
    c.bench_function("part_miller_loop_2", |b| {
        b.iter(|| {
            ark_bn254::Bn254::multi_miller_loop(
                [black_box(p), black_box(p)],
                [black_box(q), black_box(q)],
            )
        })
    });
    let ml = ark_bn254::Bn254::multi_miller_loop([p], [q]);
    c.bench_function("part_final_exp", |b| {
        b.iter(|| ark_bn254::Bn254::final_exponentiation(black_box(ml)))
    });
}

criterion_group!(benches, bench_bn254, bench_pairing_parts);
criterion_main!(benches);
