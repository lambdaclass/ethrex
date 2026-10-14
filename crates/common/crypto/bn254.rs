//! BN254 (alt_bn128) arithmetic behind the ECADD, ECMUL and ECPAIRING precompiles.
//!
//! Field multiplication, the Miller loop and the final exponentiation come from
//! `ark-bn254`. This module replaces the parts that dominate a single precompile
//! call: decoding the inputs, the field inversion needed to return an affine
//! point, the scalar multiplication, and the G2 subgroup check.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine, G2Projective};
use ark_ec::pairing::Pairing;
use ark_ec::{AffineRepr, PrimeGroup};
use ark_ff::{AdditiveGroup, BigInt, BigInteger, Field, One, PrimeField, Zero};

use crate::CryptoError;

/// An affine G1 point; `None` is the point at infinity, encoded as `(0, 0)`.
type G1 = Option<(Fq, Fq)>;

/// Curve coefficient `b` of `y^2 = x^3 + b`.
const COEFF_B: Fq = ark_ff::MontFp!("3");

/// Decodes a 32-byte big-endian field element, rejecting values `>= p`.
#[inline]
fn decode_fq(bytes: &[u8]) -> Option<Fq> {
    let limbs = be_bytes_to_limbs(bytes)?;
    Fq::from_bigint(BigInt(limbs))
}

#[inline]
fn be_bytes_to_limbs(bytes: &[u8]) -> Option<[u64; 4]> {
    let bytes: &[u8; 32] = bytes.try_into().ok()?;
    let mut limbs = [0u64; 4];
    for (i, limb) in limbs.iter_mut().enumerate() {
        let start = 24 - 8 * i;
        let mut chunk = [0u8; 8];
        chunk.copy_from_slice(&bytes[start..start + 8]);
        *limb = u64::from_be_bytes(chunk);
    }
    Some(limbs)
}

#[inline]
fn encode_fq(value: &Fq, out: &mut [u8]) {
    let limbs = value.into_bigint().0;
    for (i, limb) in limbs.iter().enumerate() {
        let start = 24 - 8 * i;
        out[start..start + 8].copy_from_slice(&limb.to_be_bytes());
    }
}

/// Decodes a 64-byte `x || y` point, checking that it lies on the curve.
/// G1 has cofactor 1, so every point on the curve is in the subgroup.
fn decode_g1(bytes: &[u8]) -> Result<G1, CryptoError> {
    let (Some(x_bytes), Some(y_bytes)) = (bytes.get(..32), bytes.get(32..64)) else {
        return Err(CryptoError::InvalidInput("G1 point must be 64 bytes"));
    };
    let (Some(x), Some(y)) = (decode_fq(x_bytes), decode_fq(y_bytes)) else {
        return Err(CryptoError::InvalidPoint(
            "G1 coordinate exceeds field modulus",
        ));
    };
    if x.is_zero() && y.is_zero() {
        return Ok(None);
    }
    if y.square() != x.square() * x + COEFF_B {
        return Err(CryptoError::InvalidPoint("G1 point not on curve"));
    }
    Ok(Some((x, y)))
}

fn encode_g1(point: &G1) -> [u8; 64] {
    let mut out = [0u8; 64];
    if let Some((x, y)) = point {
        encode_fq(x, &mut out[..32]);
        encode_fq(y, &mut out[32..]);
    }
    out
}

/// ECADD: `p1 + p2` for 64-byte encoded points.
pub(crate) fn g1_add(p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
    let a = decode_g1(p1)?;
    let b = decode_g1(p2)?;
    Ok(encode_g1(&add_affine(a, b)))
}

/// ECMUL: `k * p` for a 64-byte encoded point and a 32-byte big-endian scalar.
pub(crate) fn g1_mul(point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
    let Some(scalar) = scalar.get(..32) else {
        return Err(CryptoError::InvalidInput("scalar must be 32 bytes"));
    };
    let Some((x, y)) = decode_g1(point)? else {
        return Ok([0u8; 64]);
    };
    // The group order is prime, so multiplying by `k` equals multiplying by
    // `k mod r`.
    let k = Fr::from_be_bytes_mod_order(scalar);
    if k.is_zero() {
        return Ok([0u8; 64]);
    }
    // GLV pays a fixed cost for its table of odd multiples (two field inversions in
    // all), which a short scalar does not win back: for k = 2 double-and-add takes
    // 0.95 us against 3.3 us. The two break even at about 19 bits for dense scalars
    // (the worst case for double-and-add), so the cut-off at 16 bits leaves a margin.
    if k.into_bigint().num_bits() <= SMALL_SCALAR_BITS {
        return Ok(encode_g1(&mul_double_and_add(x, y, k)));
    }
    Ok(encode_g1(&mul_glv(x, y, k)))
}

/// Scalars up to this many bits go through double-and-add instead of GLV.
const SMALL_SCALAR_BITS: u32 = 16;

/// ECPAIRING: `true` when the product of the pairings of all pairs is one.
/// Each pair is a 64-byte G1 point and a 128-byte G2 point (imaginary parts
/// first, as the precompile encodes them).
pub(crate) fn pairing_check(pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
    let mut g1_points = Vec::with_capacity(pairs.len());
    let mut g2_points = Vec::with_capacity(pairs.len());
    for (g1_bytes, g2_bytes) in pairs {
        let g1 = match decode_g1(g1_bytes)? {
            Some((x, y)) => G1Affine::new_unchecked(x, y),
            None => G1Affine::identity(),
        };
        let g2 = decode_g2(g2_bytes)?;
        if g1.is_zero() || g2.is_zero() {
            // A pair with the identity on either side contributes one.
            continue;
        }
        g1_points.push(g1);
        g2_points.push(g2);
    }
    if g1_points.is_empty() {
        return Ok(true);
    }
    Ok(Bn254::multi_pairing(g1_points, g2_points).0.is_one())
}

/// Decodes a 128-byte G2 point and checks that it lies on the twist and in the
/// prime-order subgroup.
fn decode_g2(bytes: &[u8]) -> Result<G2Affine, CryptoError> {
    let mut coordinates = [Fq::ZERO; 4];
    for (i, coordinate) in coordinates.iter_mut().enumerate() {
        let Some(chunk) = bytes.get(32 * i..32 * (i + 1)) else {
            return Err(CryptoError::InvalidInput("G2 point must be 128 bytes"));
        };
        *coordinate = decode_fq(chunk).ok_or(CryptoError::InvalidPoint(
            "G2 coordinate exceeds field modulus",
        ))?;
    }
    let [x_im, x_re, y_im, y_re] = coordinates;
    if x_im.is_zero() && x_re.is_zero() && y_im.is_zero() && y_re.is_zero() {
        return Ok(G2Affine::identity());
    }
    let point = G2Affine::new_unchecked(Fq2::new(x_re, x_im), Fq2::new(y_re, y_im));
    if !point.is_on_curve() {
        return Err(CryptoError::InvalidPoint("G2 point not on curve"));
    }
    if !g2_in_subgroup(&point) {
        return Err(CryptoError::InvalidPoint("G2 point not in subgroup"));
    }
    Ok(point)
}

/// BN254 seed `x0` (the curve parameter `u`).
const SEED: u64 = 4_965_661_367_192_848_881;

/// `(u + 9)^((p - 1) / 3)` and `(u + 9)^((p - 1) / 2)`, the coefficients of the
/// untwist-Frobenius-twist endomorphism psi on E'(Fq2).
const PSI_X: Fq2 = Fq2::new(
    ark_ff::MontFp!(
        "21575463638280843010398324269430826099269044274347216827212613867836435027261"
    ),
    ark_ff::MontFp!(
        "10307601595873709700152284273816112264069230130616436755625194854815875713954"
    ),
);
const PSI_Y: Fq2 = Fq2::new(
    ark_ff::MontFp!("2821565182194536844548159561693502659359617185244120367078079554186484126554"),
    ark_ff::MontFp!("3505843767911556378687030309984248845540243509899259641013678093033130930403"),
);

/// psi(x, y) = (conj(x) * PSI_X, conj(y) * PSI_Y), applied to Jacobian
/// coordinates (conjugation is a field automorphism, so Z is conjugated too).
fn psi(point: &G2Projective) -> G2Projective {
    let mut result = *point;
    result.x.conjugate_in_place();
    result.x *= PSI_X;
    result.y.conjugate_in_place();
    result.y *= PSI_Y;
    result.z.conjugate_in_place();
    result
}

/// G2 membership for a point already on the twist, using
/// `[x0 + 1]P + psi([x0]P) + psi^2([x0]P) == psi^3([2 x0]P)` ("Fast subgroup
/// membership testing for G1, G2 and GT on pairing-friendly curves", Dai, Lin,
/// Zhao and Zhou, 2022, section 5.1). It needs one 63-bit scalar
/// multiplication instead of the 127-bit `[6 x0^2]P` of the generic check.
fn g2_in_subgroup(point: &G2Affine) -> bool {
    let p = point.into_group();
    let x0_p = p.mul_bigint([SEED]);
    let psi_x0_p = psi(&x0_p);
    let psi2_x0_p = psi(&psi_x0_p);
    let lhs = x0_p + p + psi_x0_p + psi2_x0_p;
    let rhs = psi(&psi2_x0_p).double();
    lhs == rhs
}

/// Affine addition with a single field inversion.
fn add_affine(a: G1, b: G1) -> G1 {
    let (x1, y1) = match a {
        Some(point) => point,
        None => return b,
    };
    let (x2, y2) = match b {
        Some(point) => point,
        None => return a,
    };
    if x1 == x2 {
        if y1 != y2 || y1.is_zero() {
            // `p + (-p)`: the result is the point at infinity.
            return None;
        }
        return double_affine(x1, y1);
    }
    let lambda = (y2 - y1) * inverse(&(x2 - x1))?;
    let x3 = lambda.square() - x1 - x2;
    let y3 = lambda * (x1 - x3) - y1;
    Some((x3, y3))
}

fn double_affine(x: Fq, y: Fq) -> G1 {
    let x_squared = x.square();
    let numerator = x_squared.double() + x_squared;
    let lambda = numerator * inverse(&y.double())?;
    let x3 = lambda.square() - x.double();
    let y3 = lambda * (x - x3) - y;
    Some((x3, y3))
}

/// A point in Jacobian coordinates `(X, Y, Z)` representing `(X/Z^2, Y/Z^3)`;
/// `Z = 0` is the point at infinity.
#[derive(Clone, Copy)]
struct Jacobian {
    x: Fq,
    y: Fq,
    z: Fq,
}

impl Jacobian {
    const INFINITY: Self = Self {
        x: Fq::ONE,
        y: Fq::ONE,
        z: Fq::ZERO,
    };

    fn from_affine(x: Fq, y: Fq) -> Self {
        Self { x, y, z: Fq::ONE }
    }

    fn is_infinity(&self) -> bool {
        self.z.is_zero()
    }

    /// Doubling for `a = 0` curves (dbl-2009-l).
    fn double(&self) -> Self {
        if self.is_infinity() {
            return *self;
        }
        let a = self.x.square();
        let b = self.y.square();
        let c = b.square();
        let d = ((self.x + b).square() - a - c).double();
        let e = a.double() + a;
        let f = e.square();
        let x3 = f - d.double();
        let y3 = e * (d - x3) - c.double().double().double();
        let z3 = (self.y * self.z).double();
        Self {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    /// Mixed addition with an affine point (madd-2007-bl).
    fn add_affine(&self, x2: Fq, y2: Fq) -> Self {
        if self.is_infinity() {
            return Self::from_affine(x2, y2);
        }
        let z1z1 = self.z.square();
        let u2 = x2 * z1z1;
        let s2 = y2 * self.z * z1z1;
        let h = u2 - self.x;
        let r = (s2 - self.y).double();
        if h.is_zero() {
            if r.is_zero() {
                return self.double();
            }
            return Self::INFINITY;
        }
        let hh = h.square();
        let i = hh.double().double();
        let j = h * i;
        let v = self.x * i;
        let x3 = r.square() - j - v.double();
        let y3 = r * (v - x3) - (self.y * j).double();
        let z3 = (self.z + h).square() - z1z1 - hh;
        Self {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    /// Full Jacobian addition (add-2007-bl), used only to build the table.
    fn add(&self, other: &Self) -> Self {
        if self.is_infinity() {
            return *other;
        }
        if other.is_infinity() {
            return *self;
        }
        let z1z1 = self.z.square();
        let z2z2 = other.z.square();
        let u1 = self.x * z2z2;
        let u2 = other.x * z1z1;
        let s1 = self.y * other.z * z2z2;
        let s2 = other.y * self.z * z1z1;
        let h = u2 - u1;
        let r = (s2 - s1).double();
        if h.is_zero() {
            if r.is_zero() {
                return self.double();
            }
            return Self::INFINITY;
        }
        let i = h.double().square();
        let j = h * i;
        let v = u1 * i;
        let x3 = r.square() - j - v.double();
        let y3 = r * (v - x3) - (s1 * j).double();
        let z3 = ((self.z + other.z).square() - z1z1 - z2z2) * h;
        Self {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    fn to_affine(self) -> G1 {
        let z_inv = inverse(&self.z)?;
        let z_inv_squared = z_inv.square();
        Some((self.x * z_inv_squared, self.y * z_inv_squared * z_inv))
    }
}

/// Window width of the wNAF recoding: digits are odd and in `(-2^(W-1), 2^(W-1))`.
const WNAF_WIDTH: u32 = 5;
/// Number of odd multiples `P, 3P, ..., (2^(W-1) - 1)P` in a table.
const TABLE_SIZE: usize = 1 << (WNAF_WIDTH - 2);
/// GLV halves of a BN254 scalar are below 2^128; wNAF adds at most one digit.
const MAX_WNAF_DIGITS: usize = 130;

/// Scalar multiplication with the GLV endomorphism and interleaved wNAF.
fn mul_glv(x: Fq, y: Fq, k: Fr) -> G1 {
    let Some(((k1_positive, k1), (k2_positive, k2))) = glv_split(&k.into_bigint().0) else {
        // Unreachable for these lattice constants; keep a correct fallback.
        return mul_double_and_add(x, y, k);
    };

    let table = odd_multiples(x, y);

    let mut digits1 = [0i8; MAX_WNAF_DIGITS];
    let mut digits2 = [0i8; MAX_WNAF_DIGITS];
    let len1 = wnaf(k1, &mut digits1);
    let len2 = wnaf(k2, &mut digits2);

    let mut acc = Jacobian::INFINITY;
    for i in (0..len1.max(len2)).rev() {
        acc = acc.double();
        let d1 = digits1[i];
        if d1 != 0 {
            let (px, py) = table[usize::from(d1.unsigned_abs() >> 1)];
            let negate = (d1 < 0) == k1_positive;
            acc = acc.add_affine(px, if negate { -py } else { py });
        }
        let d2 = digits2[i];
        if d2 != 0 {
            let (px, py) = table[usize::from(d2.unsigned_abs() >> 1)];
            let negate = (d2 < 0) == k2_positive;
            // phi(x, y) = (beta * x, y) maps the table for P to the table for phi(P).
            acc = acc.add_affine(GLV_BETA * px, if negate { -py } else { py });
        }
    }
    acc.to_affine()
}

/// Plain double-and-add, for short scalars and in case the GLV split ever exceeds 128 bits.
fn mul_double_and_add(x: Fq, y: Fq, k: Fr) -> G1 {
    let mut acc = Jacobian::INFINITY;
    for bit in ark_ff::BitIteratorBE::without_leading_zeros(k.into_bigint()) {
        acc = acc.double();
        if bit {
            acc = acc.add_affine(x, y);
        }
    }
    acc.to_affine()
}

/// Babai rounding constants for the GLV lattice of `ark_bn254::g1::Config`:
/// `floor(2^256 * |n22| / r)` and `floor(2^256 * n12 / r)`. Rounding error only
/// changes which lattice vector is subtracted, so `k = k1 + lambda * k2 (mod r)`
/// holds exactly and both halves stay below 2^128.
const GLV_G1: [u64; 3] = [0x5398_fd03_00ff_6565, 0x4cce_f014_a773_d2d2, 0x2];
const GLV_G2: [u64; 2] = [0xd91d_232e_c7e0_b3d7, 0x2];
/// `|n11|`, `n12` and `|n22|` of the same lattice basis.
const GLV_N11: u128 = 0x6f4d_8248_eeb8_59fc_8211_bbeb_7d4f_1128;
const GLV_N12: u128 = 0x89d3_2568_94d2_13e3;
const GLV_N22: u128 = 0x6f4d_8248_eeb8_59fd_0be4_e154_1221_250b;
/// The cube root of unity in `Fq` with `(beta * x, y) = lambda * (x, y)` for the
/// `lambda` this lattice is built for. Pinned here instead of read from arkworks, so
/// the two cannot drift apart if a later arkworks release picks the other root.
const GLV_BETA: Fq = ark_ff::MontFp!(
    "21888242871839275220042445260109153167277707414472061641714758635765020556616"
);

/// Splits `k < r` into signed halves `(positive, |k1|), (positive, |k2|)` with
/// `k = k1 + lambda * k2 (mod r)`.
fn glv_split(k: &[u64; 4]) -> Option<((bool, u128), (bool, u128))> {
    let c1 = mul_high_256(k, &GLV_G1)?;
    let c2 = mul_high_256(k, &GLV_G2)?;
    // k1 = k - c1 * |n11| - c2 * n12, k2 = c1 * n12 - c2 * |n22|, both
    // computed modulo 2^256 and read back as two's complement.
    let k1 = sub_256(&sub_256(k, &mul_128(c1, GLV_N11)), &mul_128(c2, GLV_N12));
    let k2 = sub_256(&mul_128(c1, GLV_N12), &mul_128(c2, GLV_N22));
    Some((signed_128(&k1)?, signed_128(&k2)?))
}

/// `(a * b) >> 256` for a 256-bit `a`, if it fits in 128 bits.
fn mul_high_256(a: &[u64; 4], b: &[u64]) -> Option<u128> {
    let mut product = [0u64; 8];
    for (i, &ai) in a.iter().enumerate() {
        let mut carry = 0u128;
        for (j, &bj) in b.iter().enumerate() {
            let t = u128::from(ai) * u128::from(bj) + u128::from(product[i + j]) + carry;
            product[i + j] = t as u64;
            carry = t >> 64;
        }
        product[i + b.len()] = carry as u64;
    }
    if product[6] != 0 || product[7] != 0 {
        return None;
    }
    Some(u128::from(product[4]) | (u128::from(product[5]) << 64))
}

/// Full 256-bit product of two 128-bit values.
fn mul_128(a: u128, b: u128) -> [u64; 4] {
    let (a0, a1) = (a as u64, (a >> 64) as u64);
    let (b0, b1) = (b as u64, (b >> 64) as u64);
    let p00 = u128::from(a0) * u128::from(b0);
    let p01 = u128::from(a0) * u128::from(b1);
    let p10 = u128::from(a1) * u128::from(b0);
    let p11 = u128::from(a1) * u128::from(b1);
    let mid = (p00 >> 64) + (p01 & u128::from(u64::MAX)) + (p10 & u128::from(u64::MAX));
    let high = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    [p00 as u64, mid as u64, high as u64, (high >> 64) as u64]
}

fn sub_256(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let mut out = [0u64; 4];
    let mut borrow = false;
    for i in 0..4 {
        let (d1, b1) = a[i].overflowing_sub(b[i]);
        let (d2, b2) = d1.overflowing_sub(u64::from(borrow));
        out[i] = d2;
        borrow = b1 || b2;
    }
    out
}

/// Reads a two's complement 256-bit value as `(non_negative, magnitude)` if
/// the magnitude fits in 128 bits.
fn signed_128(value: &[u64; 4]) -> Option<(bool, u128)> {
    let negative = value[3] >> 63 == 1;
    let magnitude = if negative {
        sub_256(&[0; 4], value)
    } else {
        *value
    };
    if magnitude[2] != 0 || magnitude[3] != 0 {
        return None;
    }
    Some((
        !negative,
        u128::from(magnitude[0]) | (u128::from(magnitude[1]) << 64),
    ))
}

/// Writes the width-`WNAF_WIDTH` NAF of `k` (least significant digit first)
/// and returns the number of digits.
fn wnaf(mut k: u128, digits: &mut [i8; MAX_WNAF_DIGITS]) -> usize {
    const WINDOW: u128 = 1 << WNAF_WIDTH;
    const HALF: u128 = 1 << (WNAF_WIDTH - 1);
    // `k` is below 2^128 but adding back a negative digit can carry into
    // bit 128, so track that carry separately.
    let mut carry = false;
    let mut len = 0;
    while k != 0 || carry {
        let digit = if k & 1 == 1 {
            let window = k & (WINDOW - 1);
            if window >= HALF {
                let (sum, overflow) = k.overflowing_add(WINDOW - window);
                k = sum;
                carry |= overflow;
                // `window - WINDOW` is in `(-2^(W-1), 0)`.
                let magnitude = i8::try_from(WINDOW - window);
                debug_assert!(magnitude.is_ok(), "wNAF digit does not fit in i8");
                -magnitude.unwrap_or(0)
            } else {
                k -= window;
                let digit = i8::try_from(window);
                debug_assert!(digit.is_ok(), "wNAF digit does not fit in i8");
                digit.unwrap_or(0)
            }
        } else {
            0
        };
        digits[len] = digit;
        len += 1;
        k >>= 1;
        if carry {
            k |= 1 << 127;
            carry = false;
        }
    }
    len
}

/// Affine `[P, 3P, 5P, ..., (2*TABLE_SIZE - 1)P]`, normalized with a single
/// inversion (Montgomery's trick).
fn odd_multiples(x: Fq, y: Fq) -> [(Fq, Fq); TABLE_SIZE] {
    let base = Jacobian::from_affine(x, y);
    let double = base.double();
    let mut jacobian = [base; TABLE_SIZE];
    for i in 1..TABLE_SIZE {
        jacobian[i] = jacobian[i - 1].add(&double);
    }

    // A multiple of P can only be infinity if P has small order, which the
    // prime-order group rules out; fall back to per-point conversion anyway.
    debug_assert!(
        !jacobian.iter().any(Jacobian::is_infinity),
        "odd multiple of a G1 point is infinity"
    );
    if jacobian.iter().any(Jacobian::is_infinity) {
        let mut table = [(x, y); TABLE_SIZE];
        for (entry, point) in table.iter_mut().zip(jacobian.iter()) {
            if let Some(affine) = point.to_affine() {
                *entry = affine;
            }
        }
        return table;
    }

    let mut prefix = [Fq::ONE; TABLE_SIZE];
    let mut running = Fq::ONE;
    for (i, point) in jacobian.iter().enumerate() {
        prefix[i] = running;
        running *= point.z;
    }
    let inv = inverse(&running);
    debug_assert!(inv.is_some(), "product of nonzero z coordinates is zero");
    let mut inv = inv.unwrap_or(Fq::ZERO);
    let mut table = [(x, y); TABLE_SIZE];
    for i in (0..TABLE_SIZE).rev() {
        let z_inv = inv * prefix[i];
        inv *= jacobian[i].z;
        let z_inv_squared = z_inv.square();
        table[i] = (
            jacobian[i].x * z_inv_squared,
            jacobian[i].y * z_inv_squared * z_inv,
        );
    }
    table
}

/// Field inversion; `None` for zero.
fn inverse(value: &Fq) -> Option<Fq> {
    if value.is_zero() {
        return None;
    }
    let inv = safegcd::inverse(value.into_bigint().0);
    Fq::from_bigint(BigInt(inv))
}

/// Variable-time modular inversion modulo the BN254 base field prime using
/// Bernstein-Yang divsteps in batches of 62 ("safegcd", with the variable-time
/// batching from "Fast constant-time gcd computation and modular inversion",
/// Bernstein and Yang, 2019). The input is a public precompile argument, so
/// variable time is fine, and it is several times faster than the binary
/// extended Euclidean algorithm.
mod safegcd {
    const M62: u64 = u64::MAX >> 2;

    /// The modulus as five signed 62-bit limbs.
    const MODULUS: [i64; 5] = to_signed62([
        0x3c20_8c16_d87c_fd47,
        0x9781_6a91_6871_ca8d,
        0xb850_45b6_8181_585d,
        0x3064_4e72_e131_a029,
    ]);

    /// `MODULUS^-1 mod 2^62`.
    const MODULUS_INV62: u64 = inverse_mod_2_64(0x3c20_8c16_d87c_fd47) & M62;

    const fn inverse_mod_2_64(value: u64) -> u64 {
        // Newton iteration doubles the number of correct low bits each step,
        // starting from 3 correct bits for any odd value.
        let mut inv = value;
        let mut i = 0;
        while i < 5 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(value.wrapping_mul(inv)));
            i += 1;
        }
        inv
    }
    const fn to_signed62(a: [u64; 4]) -> [i64; 5] {
        [
            (a[0] & M62) as i64,
            ((a[0] >> 62 | a[1] << 2) & M62) as i64,
            ((a[1] >> 60 | a[2] << 4) & M62) as i64,
            ((a[2] >> 58 | a[3] << 6) & M62) as i64,
            (a[3] >> 56) as i64,
        ]
    }
    fn from_signed62(v: [i64; 5]) -> [u64; 4] {
        let v = v.map(|limb| limb as u64);
        [
            v[0] | v[1] << 62,
            v[1] >> 2 | v[2] << 60,
            v[2] >> 4 | v[3] << 58,
            v[3] >> 6 | v[4] << 56,
        ]
    }

    /// The 2x2 transition matrix of 62 divsteps, scaled by 2^62.
    struct Transition {
        u: i64,
        v: i64,
        q: i64,
        r: i64,
    }

    /// Returns `value^-1 mod p` for `0 < value < p`, as little-endian limbs.
    pub(super) fn inverse(value: [u64; 4]) -> [u64; 4] {
        let mut d = [0i64; 5];
        let mut e = [1i64, 0, 0, 0, 0];
        let mut f = MODULUS;
        let mut g = to_signed62(value);
        let mut len = 5usize;
        // eta = -delta, with delta starting at 1.
        let mut eta: i64 = -1;

        loop {
            let (t, next_eta) = divsteps_62(eta, f[0], g[0]);
            eta = next_eta;
            update_de(&mut d, &mut e, &t);
            update_fg(len, &mut f, &mut g, &t);

            if g[0] == 0 && g[1..len].iter().all(|&limb| limb == 0) {
                break;
            }

            // Shrink the working length once the top limbs of f and g are
            // both just sign extension.
            let fn_ = f[len - 1];
            let gn = g[len - 1];
            let not_shrinkable = len < 2 || (fn_ ^ (fn_ >> 63)) != 0 || (gn ^ (gn >> 63)) != 0;
            if !not_shrinkable {
                f[len - 2] |= fn_.wrapping_shl(62);
                g[len - 2] |= gn.wrapping_shl(62);
                len -= 1;
            }
        }

        // g is zero and f is +-gcd = +-1; d holds +-value^-1.
        from_signed62(normalize(d, f[len - 1]))
    }

    /// Runs 62 divsteps on the low 62 bits of `f` and `g` (both odd/any) and
    /// returns the transition matrix and the new eta.
    fn divsteps_62(mut eta: i64, f0: i64, g0: i64) -> (Transition, i64) {
        let (mut u, mut v, mut q, mut r): (u64, u64, u64, u64) = (1, 0, 0, 1);
        let mut f = f0 as u64;
        let mut g = g0 as u64;
        let mut i: u32 = 62;

        loop {
            // Count zeros only up to i with a sentinel bit.
            let zeros = (g | (u64::MAX << i)).trailing_zeros();
            // Each of those divsteps just halves g.
            g >>= zeros;
            u <<= zeros;
            v <<= zeros;
            eta -= i64::from(zeros);
            i -= zeros;
            if i == 0 {
                break;
            }
            let limit: u32;
            let mask: u64;
            let w: u64;
            if eta < 0 {
                // Negate eta and replace (f, g) with (g, -f).
                eta = -eta;
                let tmp = f;
                f = g;
                g = tmp.wrapping_neg();
                let tmp = u;
                u = q;
                q = tmp.wrapping_neg();
                let tmp = v;
                v = r;
                r = tmp.wrapping_neg();
                // Cancel up to 6 bits of g, but no more than i, and no more
                // than eta + 1 before eta's sign flips again.
                limit = ((eta + 1) as u32).min(i);
                mask = (u64::MAX >> (64 - limit)) & 63;
                w = f
                    .wrapping_mul(g)
                    .wrapping_mul(f.wrapping_mul(f).wrapping_sub(2))
                    & mask;
            } else {
                // Cancel up to 4 bits of g.
                limit = ((eta + 1) as u32).min(i);
                mask = (u64::MAX >> (64 - limit)) & 15;
                let w0 = f.wrapping_add(((f.wrapping_add(1)) & 4) << 1);
                w = w0.wrapping_neg().wrapping_mul(g) & mask;
            }
            g = g.wrapping_add(f.wrapping_mul(w));
            q = q.wrapping_add(u.wrapping_mul(w));
            r = r.wrapping_add(v.wrapping_mul(w));
        }

        (
            Transition {
                u: u as i64,
                v: v as i64,
                q: q as i64,
                r: r as i64,
            },
            eta,
        )
    }

    /// `[d, e] = t * [d, e] / 2^62 (mod p)`, keeping limbs in range.
    fn update_de(d: &mut [i64; 5], e: &mut [i64; 5], t: &Transition) {
        let (u, v, q, r) = (t.u, t.v, t.q, t.r);
        // Add multiples of the modulus so the low 62 bits become zero; start
        // with corrections for negative d and e.
        let sd = d[4] >> 63;
        let se = e[4] >> 63;
        let mut md = (u & sd).wrapping_add(v & se);
        let mut me = (q & sd).wrapping_add(r & se);

        let mut cd = i128::from(u) * i128::from(d[0]) + i128::from(v) * i128::from(e[0]);
        let mut ce = i128::from(q) * i128::from(d[0]) + i128::from(r) * i128::from(e[0]);

        md = md.wrapping_sub(
            (MODULUS_INV62
                .wrapping_mul(cd as u64)
                .wrapping_add(md as u64)
                & M62) as i64,
        );
        me = me.wrapping_sub(
            (MODULUS_INV62
                .wrapping_mul(ce as u64)
                .wrapping_add(me as u64)
                & M62) as i64,
        );

        cd += i128::from(MODULUS[0]) * i128::from(md);
        ce += i128::from(MODULUS[0]) * i128::from(me);
        cd >>= 62;
        ce >>= 62;

        for i in 1..5 {
            cd += i128::from(u) * i128::from(d[i]) + i128::from(v) * i128::from(e[i]);
            ce += i128::from(q) * i128::from(d[i]) + i128::from(r) * i128::from(e[i]);
            cd += i128::from(MODULUS[i]) * i128::from(md);
            ce += i128::from(MODULUS[i]) * i128::from(me);
            d[i - 1] = (cd as u64 & M62) as i64;
            e[i - 1] = (ce as u64 & M62) as i64;
            cd >>= 62;
            ce >>= 62;
        }
        d[4] = cd as i64;
        e[4] = ce as i64;
    }

    /// `[f, g] = t * [f, g] / 2^62` over the first `len` limbs.
    fn update_fg(len: usize, f: &mut [i64; 5], g: &mut [i64; 5], t: &Transition) {
        let (u, v, q, r) = (t.u, t.v, t.q, t.r);
        let mut cf = i128::from(u) * i128::from(f[0]) + i128::from(v) * i128::from(g[0]);
        let mut cg = i128::from(q) * i128::from(f[0]) + i128::from(r) * i128::from(g[0]);
        cf >>= 62;
        cg >>= 62;
        for i in 1..len {
            cf += i128::from(u) * i128::from(f[i]) + i128::from(v) * i128::from(g[i]);
            cg += i128::from(q) * i128::from(f[i]) + i128::from(r) * i128::from(g[i]);
            f[i - 1] = (cf as u64 & M62) as i64;
            g[i - 1] = (cg as u64 & M62) as i64;
            cf >>= 62;
            cg >>= 62;
        }
        f[len - 1] = cf as i64;
        g[len - 1] = cg as i64;
    }

    /// Maps `r` in `(-2p, p)` to `[0, p)`, negating first when `sign < 0`.
    fn normalize(mut r: [i64; 5], sign: i64) -> [i64; 5] {
        let m62 = (u64::MAX >> 2).cast_signed();
        let cond_add = r[4] >> 63;
        for (limb, modulus) in r.iter_mut().zip(MODULUS) {
            *limb += modulus & cond_add;
        }
        let cond_negate = sign >> 63;
        for limb in r.iter_mut() {
            *limb = (*limb ^ cond_negate) - cond_negate;
        }
        propagate(&mut r, m62);
        let cond_add = r[4] >> 63;
        for (limb, modulus) in r.iter_mut().zip(MODULUS) {
            *limb += modulus & cond_add;
        }
        propagate(&mut r, m62);
        r
    }

    fn propagate(r: &mut [i64; 5], m62: i64) {
        for i in 0..4 {
            r[i + 1] += r[i] >> 62;
            r[i] &= m62;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::BigInteger;
    use sha2::{Digest, Sha256};

    /// Deterministic pseudo-random 32-byte strings.
    fn random_bytes(seed: u64) -> [u8; 32] {
        Sha256::digest(seed.to_le_bytes()).into()
    }

    fn random_fq(seed: u64) -> Fq {
        Fq::from_be_bytes_mod_order(&random_bytes(seed))
    }

    fn random_point(seed: u64) -> ark_bn254::G1Affine {
        let k = Fr::from_be_bytes_mod_order(&random_bytes(seed ^ 0x5eed));
        (ark_bn254::G1Affine::generator() * k).into_affine()
    }

    fn ark_encode(p: &ark_bn254::G1Affine) -> [u8; 64] {
        if p.is_zero() {
            return [0u8; 64];
        }
        let mut out = [0u8; 64];
        encode_fq(&p.x, &mut out[..32]);
        encode_fq(&p.y, &mut out[32..]);
        out
    }

    #[test]
    fn inverse_matches_field_inverse() {
        for seed in 0..5000 {
            let a = random_fq(seed);
            assert_eq!(inverse(&a), a.inverse(), "seed {seed}");
        }
        for small in [1u64, 2, 3, 0xffff_ffff, u64::MAX] {
            let a = Fq::from(small);
            assert_eq!(inverse(&a), a.inverse());
        }
        let p_minus_one = -Fq::ONE;
        assert_eq!(inverse(&p_minus_one), p_minus_one.inverse());
        assert_eq!(inverse(&Fq::ZERO), None);
    }

    #[test]
    fn add_matches_reference() {
        for seed in 0..500 {
            let a = random_point(seed);
            let b = random_point(seed + 10_000);
            let (ea, eb) = (ark_encode(&a), ark_encode(&b));
            assert_eq!(
                g1_add(&ea, &eb).unwrap(),
                ark_encode(&(a + b).into_affine())
            );
            assert_eq!(
                g1_add(&ea, &ea).unwrap(),
                ark_encode(&(a + a).into_affine())
            );
            assert_eq!(g1_add(&ea, &ark_encode(&(-a))).unwrap(), [0u8; 64]);
            assert_eq!(g1_add(&ea, &[0u8; 64]).unwrap(), ea);
            assert_eq!(g1_add(&[0u8; 64], &eb).unwrap(), eb);
        }
        assert_eq!(g1_add(&[0u8; 64], &[0u8; 64]).unwrap(), [0u8; 64]);
    }

    #[test]
    fn mul_matches_reference() {
        let mut scalars: Vec<[u8; 32]> = vec![[0u8; 32], [0xffu8; 32]];
        for small in [1u8, 2, 3, 15, 16, 17, 31, 32, 33] {
            let mut s = [0u8; 32];
            s[31] = small;
            scalars.push(s);
        }
        let mut r = [0u8; 32];
        r.copy_from_slice(&Fr::MODULUS.to_bytes_be());
        scalars.push(r);
        let mut r_minus_one = r;
        r_minus_one[31] -= 1;
        scalars.push(r_minus_one);
        let mut r_plus_one = r;
        r_plus_one[31] += 1;
        scalars.push(r_plus_one);
        // 2^16 - 1 and 2^16: either side of SMALL_SCALAR_BITS.
        let mut below_glv = [0u8; 32];
        below_glv[30..].copy_from_slice(&[0xff, 0xff]);
        scalars.push(below_glv);
        let mut above_glv = [0u8; 32];
        above_glv[29] = 1;
        scalars.push(above_glv);
        // 2^128 - 1 and 2^127: GLV half boundaries.
        let mut low_128 = [0u8; 32];
        low_128[16..].copy_from_slice(&[0xffu8; 16]);
        scalars.push(low_128);
        let mut bit_127 = [0u8; 32];
        bit_127[16] = 0x80;
        scalars.push(bit_127);
        for seed in 0..400 {
            scalars.push(random_bytes(seed + 1_000_000));
        }
        for (i, scalar) in scalars.iter().enumerate() {
            let p = random_point(u64::try_from(i).unwrap());
            let k = Fr::from_be_bytes_mod_order(scalar);
            let expected = ark_encode(&(p * k).into_affine());
            assert_eq!(
                g1_mul(&ark_encode(&p), scalar).unwrap(),
                expected,
                "scalar #{i}"
            );
        }
        assert_eq!(g1_mul(&[0u8; 64], &[0xffu8; 32]).unwrap(), [0u8; 64]);
    }

    #[test]
    fn wnaf_reconstructs_scalar() {
        for value in [0u128, 1, 15, 16, 31, 0xffff, u128::MAX >> 1, u128::MAX] {
            let mut digits = [0i8; MAX_WNAF_DIGITS];
            let len = wnaf(value, &mut digits);
            // value == sum(digits[i] * 2^i)
            let mut total = num_like::Big::zero();
            for i in (0..len).rev() {
                total = total.shl1().add_i8(digits[i]);
                if digits[i] != 0 {
                    assert!(digits[i] % 2 != 0 && digits[i].unsigned_abs() < 16);
                }
            }
            assert_eq!(total, num_like::Big::from_u128(value), "value {value}");
        }
    }

    /// Minimal 192-bit signed accumulator for the wNAF check.
    mod num_like {
        #[derive(Debug, PartialEq, Eq, Clone, Copy)]
        pub struct Big(i128, u128); // value = hi * 2^128 + lo
        impl Big {
            pub fn zero() -> Self {
                Big(0, 0)
            }
            pub fn from_u128(v: u128) -> Self {
                Big(0, v)
            }
            pub fn shl1(self) -> Self {
                Big((self.0 << 1) | i128::from(self.1 >> 127 == 1), self.1 << 1)
            }
            pub fn add_i8(self, d: i8) -> Self {
                let d = i128::from(d);
                if d >= 0 {
                    let (lo, carry) = self.1.overflowing_add(d.unsigned_abs());
                    Big(self.0 + i128::from(carry), lo)
                } else {
                    let (lo, borrow) = self.1.overflowing_sub(d.unsigned_abs());
                    Big(self.0 - i128::from(borrow), lo)
                }
            }
        }
    }

    fn g2_encode(p: &G2Affine) -> [u8; 128] {
        let mut out = [0u8; 128];
        if p.is_zero() {
            return out;
        }
        encode_fq(&p.x.c1, &mut out[..32]);
        encode_fq(&p.x.c0, &mut out[32..64]);
        encode_fq(&p.y.c1, &mut out[64..96]);
        encode_fq(&p.y.c0, &mut out[96..]);
        out
    }

    #[test]
    fn g2_subgroup_check_matches_reference() {
        let h = G2Affine::generator();
        for seed in 0..200u64 {
            let k = Fr::from_be_bytes_mod_order(&random_bytes(seed + 7_000_000));
            let in_group = (h * k).into_affine();
            assert!(g2_in_subgroup(&in_group), "subgroup point {seed}");
            // Points on the twist taken from random x are almost never in the
            // subgroup (the cofactor is about 2^254).
            let x = Fq2::new(random_fq(seed + 8_000_000), random_fq(seed + 9_000_000));
            if let Some(off) = G2Affine::get_point_from_x_unchecked(x, seed % 2 == 0) {
                assert_eq!(
                    g2_in_subgroup(&off),
                    off.is_in_correct_subgroup_assuming_on_curve(),
                    "twist point {seed}"
                );
            }
        }
    }

    #[test]
    fn pairing_check_matches_reference() {
        let g = ark_bn254::G1Affine::generator();
        let h = G2Affine::generator();
        for seed in 0..40u64 {
            let a = Fr::from_be_bytes_mod_order(&random_bytes(seed + 1));
            let b = Fr::from_be_bytes_mod_order(&random_bytes(seed + 2));
            // e(aG, bH) * e(-abG, H) == 1
            let p1 = (g * a).into_affine();
            let q1 = (h * b).into_affine();
            let p2 = (-(g * (a * b))).into_affine();
            let (e1, f1, e2, f2) = (
                ark_encode(&p1),
                g2_encode(&q1),
                ark_encode(&p2),
                g2_encode(&h),
            );
            assert!(pairing_check(&[(&e1, &f1), (&e2, &f2)]).unwrap());
            // Break it: e(aG, bH) alone is not one.
            assert!(!pairing_check(&[(&e1, &f1)]).unwrap());
            // Identity pairs are skipped.
            assert!(pairing_check(&[(&[0u8; 64], &f1), (&e1, &[0u8; 128])]).unwrap());
        }
        assert!(pairing_check(&[]).unwrap());
        // An on-curve G2 point outside the subgroup is rejected.
        let mut seed = 0u64;
        let off = loop {
            let x = Fq2::new(random_fq(seed + 11_000_000), random_fq(seed));
            if let Some(p) = G2Affine::get_point_from_x_unchecked(x, true)
                && !p.is_in_correct_subgroup_assuming_on_curve()
            {
                break p;
            }
            seed += 1;
        };
        let e1 = ark_encode(&g);
        assert!(pairing_check(&[(&e1, &g2_encode(&off))]).is_err());
    }

    #[test]
    fn rejects_invalid_points() {
        let mut bad = [0u8; 64];
        bad[31] = 1;
        bad[63] = 1;
        assert!(g1_add(&bad, &[0u8; 64]).is_err());
        assert!(g1_mul(&bad, &[1u8; 32]).is_err());
        // A short scalar is rejected even when the point is the identity.
        assert!(g1_mul(&[0u8; 64], &[]).is_err());
        let mut too_big = [0xffu8; 64];
        too_big[32..].copy_from_slice(&[0u8; 32]);
        assert!(g1_add(&too_big, &[0u8; 64]).is_err());
    }
}
