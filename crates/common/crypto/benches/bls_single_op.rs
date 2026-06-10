//! Microbenchmarks isolating the cost of a single EIP-2537 G1/G2 MSM call at
//! k=1 (one point), which is what the `uncachable` benchmarkoor tests exercise.
//!
//! Goal: find where the per-call time goes — full precompile path vs the G2
//! subgroup check alone vs the scalar multiplication alone. The benchmarkoor
//! `uncachable[g2msm]` test showed ~93us/call for ethrex vs ~14us for
//! nethermind; this splits that number into its parts.

use blst::{
    blst_bendian_from_fp, blst_fp, blst_p1, blst_p1_affine, blst_p1_affine_in_g1,
    blst_p1_affine_on_curve, blst_p1_from_affine, blst_p1_generator, blst_p1_mult,
    blst_p1_to_affine, blst_p2, blst_p2_affine, blst_p2_affine_in_g2, blst_p2_affine_on_curve,
    blst_p2_from_affine, blst_p2_generator, blst_p2_mult, blst_p2_to_affine, blst_scalar,
    blst_scalar_from_bendian,
};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ethrex_crypto::{Crypto, NativeCrypto};

fn scalar_bytes(seed: u64) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[24..].copy_from_slice(&(seed | 1).to_be_bytes());
    s
}

fn read_scalar(bytes: &[u8; 32]) -> blst_scalar {
    let mut out = blst_scalar::default();
    unsafe { blst_scalar_from_bendian(&mut out, bytes.as_ptr()) };
    out
}

fn fp_bytes(fp: &blst_fp) -> [u8; 48] {
    let mut out = [0u8; 48];
    unsafe { blst_bendian_from_fp(out.as_mut_ptr(), fp) };
    out
}

// A valid (in-subgroup) G1 affine point: generator * seed.
fn g1_affine(seed: u64) -> blst_p1_affine {
    let g = unsafe { &*blst_p1_generator() };
    let mut out = blst_p1::default();
    let sc = read_scalar(&scalar_bytes(seed));
    unsafe { blst_p1_mult(&mut out, g, sc.b.as_ptr(), sc.b.len() * 8) };
    let mut aff = blst_p1_affine::default();
    unsafe { blst_p1_to_affine(&mut aff, &out) };
    aff
}

fn g2_affine(seed: u64) -> blst_p2_affine {
    let g = unsafe { &*blst_p2_generator() };
    let mut out = blst_p2::default();
    let sc = read_scalar(&scalar_bytes(seed));
    unsafe { blst_p2_mult(&mut out, g, sc.b.as_ptr(), sc.b.len() * 8) };
    let mut aff = blst_p2_affine::default();
    unsafe { blst_p2_to_affine(&mut aff, &out) };
    aff
}

#[allow(clippy::type_complexity)]
fn g1_pair(seed: u64) -> (([u8; 48], [u8; 48]), [u8; 32]) {
    let p = g1_affine(seed);
    ((fp_bytes(&p.x), fp_bytes(&p.y)), scalar_bytes(seed * 7))
}

#[allow(clippy::type_complexity)]
fn g2_pair(seed: u64) -> (([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32]) {
    let p = g2_affine(seed);
    (
        (
            fp_bytes(&p.x.fp[0]),
            fp_bytes(&p.x.fp[1]),
            fp_bytes(&p.y.fp[0]),
            fp_bytes(&p.y.fp[1]),
        ),
        scalar_bytes(seed * 7),
    )
}

// BLS12-381 subgroup order r (== Spec.Q in the benchmark). Big-endian.
const R_ORDER: [u8; 32] = [
    0x73, 0xed, 0xa7, 0x53, 0x29, 0x9d, 0x7d, 0x48, 0x33, 0x39, 0xd8, 0x08, 0x09, 0xa1, 0xd8, 0x05,
    0x53, 0xbd, 0xa4, 0x02, 0xff, 0xfe, 0x5b, 0xfe, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x01,
];

fn bench(c: &mut Criterion) {
    let crypto = NativeCrypto;

    // ── DIAGNOSTIC: the exact benchmarkoor degenerate cases ──
    // (a) scalar == group order r  => P*r == identity. Does ethrex reduce mod r?
    let mut g2_r = g2_pair(1);
    g2_r.1 = R_ORDER;
    let g2_r = [g2_r];
    c.bench_function("g2_msm_k1_scalar_is_r", |b| {
        b.iter(|| crypto.bls12_381_g2_msm(black_box(&g2_r)).unwrap())
    });
    // (b) input point == infinity (0,0,0,0), as the uncachable test degenerates to.
    let g2_inf = [(
        ([0u8; 48], [0u8; 48], [0u8; 48], [0u8; 48]),
        scalar_bytes(7),
    )];
    c.bench_function("g2_msm_k1_point_is_inf", |b| {
        b.iter(|| crypto.bls12_381_g2_msm(black_box(&g2_inf)).unwrap())
    });
    let mut g1_r = g1_pair(1);
    g1_r.1 = R_ORDER;
    let g1_r = [g1_r];
    c.bench_function("g1_msm_k1_scalar_is_r", |b| {
        b.iter(|| crypto.bls12_381_g1_msm(black_box(&g1_r)).unwrap())
    });
    let g1_inf = [(([0u8; 48], [0u8; 48]), scalar_bytes(7))];
    c.bench_function("g1_msm_k1_point_is_inf", |b| {
        b.iter(|| crypto.bls12_381_g1_msm(black_box(&g1_inf)).unwrap())
    });

    // ── full precompile path (decode + on-curve + subgroup + scalar mul + encode) ──
    let g1p = [g1_pair(1)];
    c.bench_function("g1_msm_k1_full", |b| {
        b.iter(|| crypto.bls12_381_g1_msm(black_box(&g1p)).unwrap())
    });
    let g2p = [g2_pair(1)];
    c.bench_function("g2_msm_k1_full", |b| {
        b.iter(|| crypto.bls12_381_g2_msm(black_box(&g2p)).unwrap())
    });

    // ── subgroup check alone (on-curve + in_gX) ──
    let g1a = g1_affine(1);
    c.bench_function("g1_subgroup_check", |b| {
        b.iter(|| {
            let p = black_box(&g1a);
            unsafe { black_box(blst_p1_affine_on_curve(p) && blst_p1_affine_in_g1(p)) }
        })
    });
    let g2a = g2_affine(1);
    c.bench_function("g2_subgroup_check", |b| {
        b.iter(|| {
            let p = black_box(&g2a);
            unsafe { black_box(blst_p2_affine_on_curve(p) && blst_p2_affine_in_g2(p)) }
        })
    });

    // ── scalar multiplication alone ──
    let sc = read_scalar(&scalar_bytes(7));
    let g1j = {
        let mut j = blst_p1::default();
        unsafe { blst_p1_from_affine(&mut j, &g1a) };
        j
    };
    c.bench_function("g1_scalar_mul", |b| {
        b.iter(|| {
            let mut out = blst_p1::default();
            unsafe {
                blst_p1_mult(&mut out, black_box(&g1j), sc.b.as_ptr(), sc.b.len() * 8);
            }
            black_box(out)
        })
    });
    let g2j = {
        let mut j = blst_p2::default();
        unsafe { blst_p2_from_affine(&mut j, &g2a) };
        j
    };
    c.bench_function("g2_scalar_mul", |b| {
        b.iter(|| {
            let mut out = blst_p2::default();
            unsafe {
                blst_p2_mult(&mut out, black_box(&g2j), sc.b.as_ptr(), sc.b.len() * 8);
            }
            black_box(out)
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
