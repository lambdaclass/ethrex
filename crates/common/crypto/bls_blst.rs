//! blst-backed BLS12-381 (EIP-2537) operations used by [`crate::NativeCrypto`].
//!
//! The portable default implementations of these operations live on the
//! [`crate::Crypto`] trait and use the pure-Rust `bls12_381` crate. That path
//! normalizes points to affine via an exponentiation-based field inversion
//! (Fermat), which dominates the cost of the cheap G1ADD/G2ADD precompiles and
//! makes them performance outliers. `blst` uses an assembly-optimized
//! binary-GCD inversion, so we route the native (non-zkVM) path through it here.
//!
//! Inputs are unpadded big-endian coordinates (48 bytes per field element), the
//! same form the precompile layer already produces after stripping EIP-2537
//! padding. The validation semantics follow EIP-2537: canonical field check,
//! on-curve check, subgroup checks where required, and point-at-infinity
//! handling.

use blst::{
    blst_bendian_from_fp, blst_final_exp, blst_fp, blst_fp_from_bendian, blst_fp2, blst_fp12,
    blst_fp12_is_one, blst_fp12_mul, blst_map_to_g1, blst_map_to_g2, blst_miller_loop, blst_p1,
    blst_p1_add_or_double_affine, blst_p1_affine, blst_p1_affine_in_g1, blst_p1_affine_is_inf,
    blst_p1_affine_on_curve, blst_p1_from_affine, blst_p1_mult, blst_p1_to_affine,
    blst_p1s_mult_pippenger, blst_p1s_mult_pippenger_scratch_sizeof, blst_p2,
    blst_p2_add_or_double_affine, blst_p2_affine, blst_p2_affine_in_g2, blst_p2_affine_is_inf,
    blst_p2_affine_on_curve, blst_p2_from_affine, blst_p2_mult, blst_p2_to_affine,
    blst_p2s_mult_pippenger, blst_p2s_mult_pippenger_scratch_sizeof, blst_scalar,
    blst_scalar_from_be_bytes,
};

use crate::provider::CryptoError;

/// Length of a serialized field element (big-endian, unpadded).
const FP_LENGTH: usize = 48;
/// Length of a scalar in bits (EIP-2537 scalars are 256-bit big-endian integers).
const SCALAR_BITS: usize = 256;

/// The BLS12-381 base field modulus `p`, big-endian, non-Montgomery form.
/// Used to reject non-canonical field elements (`blst_fp_from_bendian` does not).
const MODULUS_REPR: [u8; FP_LENGTH] = [
    0x1a, 0x01, 0x11, 0xea, 0x39, 0x7f, 0xe6, 0x9a, 0x4b, 0x1b, 0xa7, 0xb6, 0x43, 0x4b, 0xac, 0xd7,
    0x64, 0x77, 0x4b, 0x84, 0xf3, 0x85, 0x12, 0xbf, 0x67, 0x30, 0xd2, 0xa0, 0xf6, 0xb0, 0xf6, 0x24,
    0x1e, 0xab, 0xff, 0xfe, 0xb1, 0x53, 0xff, 0xff, 0xb9, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xaa, 0xab,
];

// ── low-level blst wrappers ────────────────────────────────────────────────

#[inline]
fn p1_to_affine(p: &blst_p1) -> blst_p1_affine {
    let mut out = blst_p1_affine::default();
    // SAFETY: both operands are valid, initialized blst values.
    unsafe { blst_p1_to_affine(&mut out, p) };
    out
}

#[inline]
fn p1_from_affine(p: &blst_p1_affine) -> blst_p1 {
    let mut out = blst_p1::default();
    // SAFETY: both operands are valid, initialized blst values.
    unsafe { blst_p1_from_affine(&mut out, p) };
    out
}

#[inline]
fn p2_to_affine(p: &blst_p2) -> blst_p2_affine {
    let mut out = blst_p2_affine::default();
    // SAFETY: both operands are valid, initialized blst values.
    unsafe { blst_p2_to_affine(&mut out, p) };
    out
}

#[inline]
fn p2_from_affine(p: &blst_p2_affine) -> blst_p2 {
    let mut out = blst_p2::default();
    // SAFETY: both operands are valid, initialized blst values.
    unsafe { blst_p2_from_affine(&mut out, p) };
    out
}

/// Adds two affine G1 points, returning the affine sum. `blst` handles the
/// point at infinity (encoded as `(0, 0)`) and doubling correctly.
#[inline]
fn p1_add_affine(a: &blst_p1_affine, b: &blst_p1_affine) -> blst_p1_affine {
    let a_jac = p1_from_affine(a);
    let mut sum = blst_p1::default();
    // SAFETY: all operands are valid, initialized blst values.
    unsafe { blst_p1_add_or_double_affine(&mut sum, &a_jac, b) };
    p1_to_affine(&sum)
}

/// Adds two affine G2 points, returning the affine sum.
#[inline]
fn p2_add_affine(a: &blst_p2_affine, b: &blst_p2_affine) -> blst_p2_affine {
    let a_jac = p2_from_affine(a);
    let mut sum = blst_p2::default();
    // SAFETY: all operands are valid, initialized blst values.
    unsafe { blst_p2_add_or_double_affine(&mut sum, &a_jac, b) };
    p2_to_affine(&sum)
}

#[inline]
fn p1_scalar_mul(p: &blst_p1_affine, scalar: &blst_scalar) -> blst_p1_affine {
    let p_jac = p1_from_affine(p);
    let mut out = blst_p1::default();
    // SAFETY: all operands are valid; `scalar.b` is a 32-byte little-endian buffer.
    unsafe { blst_p1_mult(&mut out, &p_jac, scalar.b.as_ptr(), scalar.b.len() * 8) };
    p1_to_affine(&out)
}

#[inline]
fn p2_scalar_mul(p: &blst_p2_affine, scalar: &blst_scalar) -> blst_p2_affine {
    let p_jac = p2_from_affine(p);
    let mut out = blst_p2::default();
    // SAFETY: all operands are valid; `scalar.b` is a 32-byte little-endian buffer.
    unsafe { blst_p2_mult(&mut out, &p_jac, scalar.b.as_ptr(), scalar.b.len() * 8) };
    p2_to_affine(&out)
}

// Pippenger MSM, called single-threaded.
//
// blst's safe `MultiPoint::mult` wrapper forks every call across blst's global
// thread pool. Inside the (already serial) EVM precompile path that fork/join is
// pure overhead: it adds per-call latency and its workers contend with the
// node's many runnable threads, which makes MSM *slower* than the serial path
// for the small point counts EIP-2537 MSMs reach. So we call blst's C
// `..._mult_pippenger` entry directly with our own scratch, keeping the fast
// serial Pippenger without spawning any threads.
//
// `points` is a non-empty slice and `scalars` holds `points.len()` consecutive
// 32-byte little-endian scalars (`SCALAR_BITS` wide). blst's array arguments are
// null-terminated lists of pointers to contiguous arrays.

#[inline]
fn p1_msm(points: &[blst_p1_affine], scalars: &[u8]) -> blst_p1 {
    let npoints = points.len();
    let p: [*const blst_p1_affine; 2] = [points.as_ptr(), core::ptr::null()];
    let s: [*const u8; 2] = [scalars.as_ptr(), core::ptr::null()];
    let mut out = blst_p1::default();
    // SAFETY: `npoints >= 2` (the single-point and empty cases are handled by
    // the caller), `scalars` is `npoints * 32` bytes, and scratch is sized by
    // blst's own helper. The helper returns a byte count; `div_ceil(8)` rounds
    // up to whole `u64` limbs so the buffer is never undersized on a target
    // where the count isn't a multiple of 8.
    unsafe {
        let mut scratch = vec![0u64; blst_p1s_mult_pippenger_scratch_sizeof(npoints).div_ceil(8)];
        blst_p1s_mult_pippenger(
            &mut out,
            p.as_ptr(),
            npoints,
            s.as_ptr(),
            SCALAR_BITS,
            scratch.as_mut_ptr(),
        );
    }
    out
}

#[inline]
fn p2_msm(points: &[blst_p2_affine], scalars: &[u8]) -> blst_p2 {
    let npoints = points.len();
    let p: [*const blst_p2_affine; 2] = [points.as_ptr(), core::ptr::null()];
    let s: [*const u8; 2] = [scalars.as_ptr(), core::ptr::null()];
    let mut out = blst_p2::default();
    // SAFETY: `npoints >= 2` (the single-point and empty cases are handled by
    // the caller), `scalars` is `npoints * 32` bytes, and scratch is sized by
    // blst's own helper. The helper returns a byte count; `div_ceil(8)` rounds
    // up to whole `u64` limbs so the buffer is never undersized on a target
    // where the count isn't a multiple of 8.
    unsafe {
        let mut scratch = vec![0u64; blst_p2s_mult_pippenger_scratch_sizeof(npoints).div_ceil(8)];
        blst_p2s_mult_pippenger(
            &mut out,
            p.as_ptr(),
            npoints,
            s.as_ptr(),
            SCALAR_BITS,
            scratch.as_mut_ptr(),
        );
    }
    out
}

// ── parsing / serialization ────────────────────────────────────────────────

/// Reads a canonical big-endian field element. Rejects values `>= p`, which
/// `blst_fp_from_bendian` would otherwise accept silently.
#[inline]
fn read_fp(input: &[u8; FP_LENGTH]) -> Result<blst_fp, CryptoError> {
    if *input >= MODULUS_REPR {
        return Err(CryptoError::InvalidInput("fp coordinate >= field modulus"));
    }
    let mut fp = blst_fp::default();
    // SAFETY: `input` is exactly FP_LENGTH bytes and `fp` is a valid blst value.
    unsafe { blst_fp_from_bendian(&mut fp, input.as_ptr()) };
    Ok(fp)
}

#[inline]
fn read_fp2(c0: &[u8; FP_LENGTH], c1: &[u8; FP_LENGTH]) -> Result<blst_fp2, CryptoError> {
    Ok(blst_fp2 {
        fp: [read_fp(c0)?, read_fp(c1)?],
    })
}

/// Parses a G1 point from affine coordinates and checks it lies on the curve.
fn decode_g1_on_curve(x: &[u8; 48], y: &[u8; 48]) -> Result<blst_p1_affine, CryptoError> {
    let point = blst_p1_affine {
        x: read_fp(x)?,
        y: read_fp(y)?,
    };
    // SAFETY: `point` is initialized. The infinity encoding (0, 0) is on-curve.
    if unsafe { !blst_p1_affine_on_curve(&point) } {
        return Err(CryptoError::InvalidPoint("G1 point not on curve"));
    }
    Ok(point)
}

/// Parses a G1 point and additionally enforces the subgroup check (required for
/// MSM and pairings per EIP-2537).
fn read_g1_subgroup(x: &[u8; 48], y: &[u8; 48]) -> Result<blst_p1_affine, CryptoError> {
    let point = decode_g1_on_curve(x, y)?;
    // SAFETY: `point` is initialized and on-curve.
    if unsafe { !blst_p1_affine_in_g1(&point) } {
        return Err(CryptoError::InvalidPoint("G1 point not in subgroup"));
    }
    Ok(point)
}

fn decode_g2_on_curve(
    x0: &[u8; 48],
    x1: &[u8; 48],
    y0: &[u8; 48],
    y1: &[u8; 48],
) -> Result<blst_p2_affine, CryptoError> {
    let point = blst_p2_affine {
        x: read_fp2(x0, x1)?,
        y: read_fp2(y0, y1)?,
    };
    // SAFETY: `point` is initialized. The infinity encoding (0, 0) is on-curve.
    if unsafe { !blst_p2_affine_on_curve(&point) } {
        return Err(CryptoError::InvalidPoint("G2 point not on curve"));
    }
    Ok(point)
}

fn read_g2_subgroup(
    x0: &[u8; 48],
    x1: &[u8; 48],
    y0: &[u8; 48],
    y1: &[u8; 48],
) -> Result<blst_p2_affine, CryptoError> {
    let point = decode_g2_on_curve(x0, x1, y0, y1)?;
    // SAFETY: `point` is initialized and on-curve.
    if unsafe { !blst_p2_affine_in_g2(&point) } {
        return Err(CryptoError::InvalidPoint("G2 point not in subgroup"));
    }
    Ok(point)
}

#[cfg(test)]
use blst::blst_scalar_from_bendian;

/// Test-only: loads a 32-byte big-endian scalar without reducing modulo the
/// subgroup order. The production MSM path uses [`read_scalar_mod_r`].
#[cfg(test)]
#[inline]
fn read_scalar(bytes: &[u8; 32]) -> blst_scalar {
    let mut out = blst_scalar::default();
    // SAFETY: `bytes` is exactly 32 bytes and `out` is a valid blst value.
    unsafe { blst_scalar_from_bendian(&mut out, bytes.as_ptr()) };
    out
}

/// Reduces a 32-byte big-endian scalar modulo the subgroup order r, returning
/// `None` when the reduced scalar is zero.
///
/// For a point in the prime-order subgroup (order r), `s·P == (s mod r)·P`, so
/// reducing is byte-identical to the EIP-2537 result while collapsing scalars
/// that are multiples of r (e.g. the group order itself) to zero — those
/// contribute the identity and can be skipped, avoiding a full ~255-bit scalar
/// multiplication. `blst_scalar_from_be_bytes` performs the reduction (unlike
/// `blst_scalar_from_bendian`) and returns `false` iff the reduced value is
/// zero, which is exactly the skip signal we want.
#[inline]
fn read_scalar_mod_r(bytes: &[u8; 32]) -> Option<blst_scalar> {
    let mut out = blst_scalar::default();
    // SAFETY: `bytes` is exactly 32 bytes and `out` is a valid blst value.
    let nonzero = unsafe { blst_scalar_from_be_bytes(&mut out, bytes.as_ptr(), 32) };
    nonzero.then_some(out)
}

#[inline]
fn fp_to_bytes(out: &mut [u8; FP_LENGTH], fp: &blst_fp) {
    // SAFETY: `out` is FP_LENGTH bytes and `fp` is a valid blst value.
    unsafe { blst_bendian_from_fp(out.as_mut_ptr(), fp) };
}

fn encode_g1(point: &blst_p1_affine) -> [u8; 96] {
    let mut out = [0u8; 96];
    let (x, y) = out.split_at_mut(FP_LENGTH);
    fp_to_bytes(x.try_into().expect("48 bytes"), &point.x);
    fp_to_bytes(y.try_into().expect("48 bytes"), &point.y);
    out
}

fn encode_g2(point: &blst_p2_affine) -> [u8; 192] {
    let mut out = [0u8; 192];
    fp_to_bytes(
        (&mut out[0..48]).try_into().expect("48 bytes"),
        &point.x.fp[0],
    );
    fp_to_bytes(
        (&mut out[48..96]).try_into().expect("48 bytes"),
        &point.x.fp[1],
    );
    fp_to_bytes(
        (&mut out[96..144]).try_into().expect("48 bytes"),
        &point.y.fp[0],
    );
    fp_to_bytes(
        (&mut out[144..192]).try_into().expect("48 bytes"),
        &point.y.fp[1],
    );
    out
}

#[inline]
fn is_zero(bytes: &[u8]) -> bool {
    bytes.iter().all(|&b| b == 0)
}

// ── public operations (consumed by NativeCrypto) ───────────────────────────

/// G1 addition. No subgroup check, per EIP-2537.
pub fn g1_add(a: ([u8; 48], [u8; 48]), b: ([u8; 48], [u8; 48])) -> Result<[u8; 96], CryptoError> {
    let pa = decode_g1_on_curve(&a.0, &a.1)?;
    let pb = decode_g1_on_curve(&b.0, &b.1)?;
    Ok(encode_g1(&p1_add_affine(&pa, &pb)))
}

/// G2 addition. No subgroup check, per EIP-2537.
pub fn g2_add(
    a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
) -> Result<[u8; 192], CryptoError> {
    let pa = decode_g2_on_curve(&a.0, &a.1, &a.2, &a.3)?;
    let pb = decode_g2_on_curve(&b.0, &b.1, &b.2, &b.3)?;
    Ok(encode_g2(&p2_add_affine(&pa, &pb)))
}

/// G1 multi-scalar multiplication. Each point is subgroup-checked; terms that
/// contribute nothing to the sum (zero scalar, or a point at infinity) are
/// skipped after validation rather than fed through a full scalar multiplication.
#[allow(clippy::type_complexity)]
pub fn g1_msm(pairs: &[(([u8; 48], [u8; 48]), [u8; 32])]) -> Result<[u8; 96], CryptoError> {
    let mut points = Vec::with_capacity(pairs.len());
    let mut scalars = Vec::with_capacity(pairs.len());

    for ((x, y), scalar_bytes) in pairs {
        let point = decode_g1_on_curve(x, y)?;
        // A point at infinity is the identity: it lies in every subgroup and
        // contributes nothing to the sum, so skip it — and its subgroup check,
        // which it would trivially pass — avoiding a full 255-bit scalar mul.
        // SAFETY: `point` is initialized and on-curve.
        if unsafe { blst_p1_affine_is_inf(&point) } {
            continue;
        }
        // Every other point must lie in the prime-order subgroup, even when
        // paired with a zero scalar (EIP-2537 input validation is unconditional).
        // SAFETY: `point` is initialized and on-curve.
        if unsafe { !blst_p1_affine_in_g1(&point) } {
            return Err(CryptoError::InvalidPoint("G1 point not in subgroup"));
        }
        // Reduce the scalar modulo the subgroup order r. The point above is
        // subgroup-checked (order r), so s·P == (s mod r)·P and reducing is
        // byte-identical to the spec. A scalar that is a multiple of r (e.g.
        // the group order) reduces to zero and contributes the identity, so it
        // is skipped here instead of running a full ~255-bit scalar mul.
        let Some(scalar) = read_scalar_mod_r(scalar_bytes) else {
            continue;
        };
        points.push(point);
        scalars.push(scalar);
    }

    if points.is_empty() {
        return Ok([0u8; 96]);
    }
    if points.len() == 1 {
        return Ok(encode_g1(&p1_scalar_mul(&points[0], &scalars[0])));
    }

    let scalar_bytes: Vec<u8> = scalars.iter().flat_map(|s| s.b).collect();
    let result = p1_to_affine(&p1_msm(&points, &scalar_bytes));
    Ok(encode_g1(&result))
}

/// G2 multi-scalar multiplication. Each point is subgroup-checked; terms that
/// contribute nothing to the sum (zero scalar, or a point at infinity) are
/// skipped after validation rather than fed through a full scalar multiplication.
#[allow(clippy::type_complexity)]
pub fn g2_msm(
    pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
) -> Result<[u8; 192], CryptoError> {
    let mut points = Vec::with_capacity(pairs.len());
    let mut scalars = Vec::with_capacity(pairs.len());

    for ((x0, x1, y0, y1), scalar_bytes) in pairs {
        let point = decode_g2_on_curve(x0, x1, y0, y1)?;
        // A point at infinity is the identity: it lies in every subgroup and
        // contributes nothing to the sum, so skip it — and its subgroup check,
        // which it would trivially pass — avoiding a full 255-bit scalar mul.
        // SAFETY: `point` is initialized and on-curve.
        if unsafe { blst_p2_affine_is_inf(&point) } {
            continue;
        }
        // Every other point must lie in the prime-order subgroup, even when
        // paired with a zero scalar (EIP-2537 input validation is unconditional).
        // SAFETY: `point` is initialized and on-curve.
        if unsafe { !blst_p2_affine_in_g2(&point) } {
            return Err(CryptoError::InvalidPoint("G2 point not in subgroup"));
        }
        // Reduce the scalar modulo the subgroup order r. The point above is
        // subgroup-checked (order r), so s·P == (s mod r)·P and reducing is
        // byte-identical to the spec. A scalar that is a multiple of r (e.g.
        // the group order) reduces to zero and contributes the identity, so it
        // is skipped here instead of running a full ~255-bit scalar mul.
        let Some(scalar) = read_scalar_mod_r(scalar_bytes) else {
            continue;
        };
        points.push(point);
        scalars.push(scalar);
    }

    if points.is_empty() {
        return Ok([0u8; 192]);
    }
    if points.len() == 1 {
        return Ok(encode_g2(&p2_scalar_mul(&points[0], &scalars[0])));
    }

    let scalar_bytes: Vec<u8> = scalars.iter().flat_map(|s| s.b).collect();
    let result = p2_to_affine(&p2_msm(&points, &scalar_bytes));
    Ok(encode_g2(&result))
}

/// Pairing check. Both points of each pair are subgroup-checked; pairs with a
/// point at infinity are no-ops and skipped after validating the other point.
#[allow(clippy::type_complexity)]
pub fn pairing_check(
    pairs: &[(
        ([u8; 48], [u8; 48]),
        ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    )],
) -> Result<bool, CryptoError> {
    let mut parsed = Vec::with_capacity(pairs.len());

    for ((g1x, g1y), (g2x0, g2x1, g2y0, g2y1)) in pairs {
        let g1_inf = is_zero(g1x) && is_zero(g1y);
        let g2_inf = is_zero(g2x0) && is_zero(g2x1) && is_zero(g2y0) && is_zero(g2y1);

        if g1_inf || g2_inf {
            // A pair with an infinity point contributes the identity, but the
            // other (non-infinity) point must still pass validation.
            if !g1_inf {
                read_g1_subgroup(g1x, g1y)?;
            }
            if !g2_inf {
                read_g2_subgroup(g2x0, g2x1, g2y0, g2y1)?;
            }
            continue;
        }

        let g1 = read_g1_subgroup(g1x, g1y)?;
        let g2 = read_g2_subgroup(g2x0, g2x1, g2y0, g2y1)?;
        parsed.push((g1, g2));
    }

    if parsed.is_empty() {
        return Ok(true);
    }

    // acc = prod miller_loop(g1_i, g2_i); pairing holds iff final_exp(acc) == 1.
    let mut acc = blst_fp12::default();
    let (first_g1, first_g2) = &parsed[0];
    // SAFETY: operands are initialized blst values; blst takes (g2, g1) order.
    unsafe { blst_miller_loop(&mut acc, first_g2, first_g1) };

    for (g1, g2) in parsed.iter().skip(1) {
        let mut ml = blst_fp12::default();
        let mut next = blst_fp12::default();
        // SAFETY: operands are initialized blst values.
        unsafe {
            blst_miller_loop(&mut ml, g2, g1);
            blst_fp12_mul(&mut next, &acc, &ml);
        }
        acc = next;
    }

    let mut result = blst_fp12::default();
    // SAFETY: `acc` and `result` are initialized blst values.
    unsafe { blst_final_exp(&mut result, &acc) };
    // SAFETY: `result` is an initialized blst value.
    Ok(unsafe { blst_fp12_is_one(&result) })
}

/// Maps a field element to a G1 point (EIP-2537 map_fp_to_g1).
pub fn fp_to_g1(fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
    let fp = read_fp(fp)?;
    let mut p = blst_p1::default();
    // SAFETY: `p` and `fp` are valid blst values; the optional aug argument is null.
    unsafe { blst_map_to_g1(&mut p, &fp, core::ptr::null()) };
    Ok(encode_g1(&p1_to_affine(&p)))
}

/// Maps an Fp2 element to a G2 point (EIP-2537 map_fp2_to_g2).
pub fn fp2_to_g2(fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
    let fp2 = read_fp2(&fp2.0, &fp2.1)?;
    let mut p = blst_p2::default();
    // SAFETY: `p` and `fp2` are valid blst values; the optional aug argument is null.
    unsafe { blst_map_to_g2(&mut p, &fp2, core::ptr::null()) };
    Ok(encode_g2(&p2_to_affine(&p)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use blst::{blst_p1_generator, blst_p2_generator};

    fn scalar_bytes(seed: u64) -> [u8; 32] {
        let mut s = [0u8; 32];
        s[24..].copy_from_slice(&(seed | 1).to_be_bytes());
        s
    }

    fn g1_xy(seed: u64) -> ([u8; 48], [u8; 48]) {
        let g = p1_to_affine(unsafe { &*blst_p1_generator() });
        let p = p1_scalar_mul(&g, &read_scalar(&scalar_bytes(seed)));
        let enc = encode_g1(&p);
        (
            enc[0..48].try_into().unwrap(),
            enc[48..96].try_into().unwrap(),
        )
    }

    fn g2_xy(seed: u64) -> ([u8; 48], [u8; 48], [u8; 48], [u8; 48]) {
        let g = p2_to_affine(unsafe { &*blst_p2_generator() });
        let p = p2_scalar_mul(&g, &read_scalar(&scalar_bytes(seed)));
        let e = encode_g2(&p);
        (
            e[0..48].try_into().unwrap(),
            e[48..96].try_into().unwrap(),
            e[96..144].try_into().unwrap(),
            e[144..192].try_into().unwrap(),
        )
    }

    // Skipping an infinity point must not change the MSM result, and an all-zero
    // (infinity-only) MSM must return the identity.
    #[test]
    fn g1_msm_skips_infinity() {
        let p = g1_xy(3);
        let inf = ([0u8; 48], [0u8; 48]);
        let with_inf = g1_msm(&[(p, scalar_bytes(5)), (inf, scalar_bytes(9))]).unwrap();
        let without = g1_msm(&[(p, scalar_bytes(5))]).unwrap();
        assert_eq!(with_inf, without, "infinity term must contribute nothing");
        assert_eq!(g1_msm(&[(inf, scalar_bytes(9))]).unwrap(), [0u8; 96]);
    }

    #[test]
    fn g2_msm_skips_infinity() {
        let p = g2_xy(3);
        let inf = ([0u8; 48], [0u8; 48], [0u8; 48], [0u8; 48]);
        let with_inf = g2_msm(&[(p, scalar_bytes(5)), (inf, scalar_bytes(9))]).unwrap();
        let without = g2_msm(&[(p, scalar_bytes(5))]).unwrap();
        assert_eq!(with_inf, without, "infinity term must contribute nothing");
        assert_eq!(g2_msm(&[(inf, scalar_bytes(9))]).unwrap(), [0u8; 192]);
    }

    // A point multiplied by the subgroup order r is the identity; with the
    // output fed back (the uncachable pattern) the next input is infinity, which
    // must be handled correctly (returns identity, not an error).
    #[test]
    fn g1_msm_point_times_order_is_identity_then_infinity() {
        const R: [u8; 32] = [
            0x73, 0xed, 0xa7, 0x53, 0x29, 0x9d, 0x7d, 0x48, 0x33, 0x39, 0xd8, 0x08, 0x09, 0xa1,
            0xd8, 0x05, 0x53, 0xbd, 0xa4, 0x02, 0xff, 0xfe, 0x5b, 0xfe, 0xff, 0xff, 0xff, 0xff,
            0x00, 0x00, 0x00, 0x01,
        ];
        let out = g1_msm(&[(g1_xy(3), R)]).unwrap();
        assert_eq!(out, [0u8; 96], "P * r == identity");
        // feed identity back as the next point
        let inf = ([0u8; 48], [0u8; 48]);
        assert_eq!(g1_msm(&[(inf, R)]).unwrap(), [0u8; 96]);
    }
}
