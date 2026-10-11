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
}

criterion_group!(benches, bench_bn254);
criterion_main!(benches);
