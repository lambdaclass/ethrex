//! LambdaVM crypto provider for ethrex's EVM.
//!
//! Implements ethrex's `Crypto` trait with LambdaVM precompile acceleration and
//! is injected into the guest binary (`Arc::new(LambdaVmCrypto)` →
//! `execution_program`). The accelerated machinery below is kept byte-for-byte
//! in sync with the LambdaVM team's copy (`crypto/ethrex-crypto` in
//! yetanotherco/lambda_vm) so both repos build the same guest — when changing
//! it, change both.
//!
//! Accelerated today:
//! - `keccak256`: a sponge over the `keccak_permute` precompile (riscv64; on
//!   host it falls back to software keccak for tests).
//! - `secp256k1_ecrecover`: the ECDSA recovery's 2-term linear combination is
//!   evaluated through the ECSM `ecsm_mul` precompile (riscv64), reconstructing
//!   the full point from x-only queries; on host / degenerate inputs it falls
//!   back to the pure-Rust `ProjectivePoint::lincomb`.
//!
//! The BLS12-381 (EIP-2537) methods are routed through the portable pure-Rust
//! `bls12_381` backend via `super::shared` — the trait defaults for those
//! return `Unsupported` when `blst` is compiled out of guest builds. Every
//! other `Crypto` method inherits the trait default, which uses vetted
//! pure-Rust crates (`ark-bn254`, `malachite`, `p256`, `sha2`) that compile to
//! the RV64IM target.
//!
//! KZG point-evaluation is unsupported in the LambdaVM guest: `kzg-rs` pulls
//! in SP1-specific symbols that do not link for this target, so the trait
//! default returns an error (blob txs execute; a point-eval precompile call
//! reverts).

use super::shared::{
    bls12_381_fp_to_g1, bls12_381_fp2_to_g2, bls12_381_g1_add, bls12_381_g1_msm, bls12_381_g2_add,
    bls12_381_g2_msm, bls12_381_pairing_check,
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_crypto::{Crypto, CryptoError};
use k256::elliptic_curve::PrimeField;
use k256::elliptic_curve::group::prime::PrimeCurveAffine;
use k256::elliptic_curve::ops::{Invert, LinearCombination, Reduce};
use k256::elliptic_curve::point::DecompressPoint;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::{AffinePoint, FieldBytes, ProjectivePoint, Scalar, U256};

// Used only by the x-only point reconstruction (riscv accelerated path + the
// host unit tests); unused on a non-test host build.
#[cfg(any(target_arch = "riscv64", test))]
use k256::elliptic_curve::sec1::FromEncodedPoint;
#[cfg(any(target_arch = "riscv64", test))]
use k256::{EncodedPoint, FieldElement};

/// LambdaVM crypto provider — inject via `Arc::new(LambdaVmCrypto)`.
#[derive(Debug)]
pub struct LambdaVmCrypto;

impl Crypto for LambdaVmCrypto {
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        let pk_bytes = ecsm_ecrecover(sig, recid, msg)?;
        Ok(self.keccak256(&pk_bytes))
    }

    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        // riscv64 guest: sponge over the keccak_permute precompile.
        #[cfg(target_arch = "riscv64")]
        return keccak256_via_lambdavm(input);
        // host (tests / non-guest): software keccak — the precompile syscall
        // isn't available off-target.
        #[cfg(not(target_arch = "riscv64"))]
        return keccak_hash(input);
    }

    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        bls12_381_g1_add(a, b)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        bls12_381_g1_msm(pairs)
    }

    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        bls12_381_g2_add(a, b)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        bls12_381_g2_msm(pairs)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        bls12_381_pairing_check(pairs)
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        bls12_381_fp_to_g1(fp)
    }

    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        bls12_381_fp2_to_g2(fp2)
    }
}

// ── ECDSA secp256k1 recovery via the ECSM precompile ────────────────────────

/// Recover the uncompressed public key bytes (X‖Y, 64 bytes) from a 64-byte
/// signature, recovery id, and 32-byte message hash. Used by the ECRECOVER
/// precompile (0x01).
///
/// Returns the raw 64-byte key; the caller is responsible for hashing it.
/// Keeping keccak out of this function lets `secp256k1_ecrecover` route the
/// hash through `self.keccak256`, which uses the keccak_permute precompile on
/// riscv64 instead of always falling back to software.
///
/// Mirrors the pure-Rust recovery in the `Crypto` trait default
/// (`pk = r⁻¹·(s·R − z·G)`), but evaluates the 2-term linear combination
/// `lincomb(G, u1, R, u2)` through the ECSM accelerator via [`ecsm_lincomb2`],
/// falling back to the software `ProjectivePoint::lincomb` whenever the
/// accelerated path declines (degenerate scalars/points, or non-riscv builds).
/// We compute the recovery directly rather than calling k256's
/// `recover_from_prehash`, which internally runs a *second* lincomb to
/// re-verify the key — doubling the ECSM ecalls for no gain here.
fn ecsm_ecrecover(sig: &[u8; 64], recid: u8, msg: &[u8; 32]) -> Result<[u8; 64], CryptoError> {
    let r_bytes = <&FieldBytes>::from(&sig[..32]);
    let s_bytes = <&FieldBytes>::from(&sig[32..]);

    // Parse r and s as scalars, rejecting values >= the curve order.
    let r: Option<Scalar> = Scalar::from_repr(*r_bytes).into();
    let s: Option<Scalar> = Scalar::from_repr(*s_bytes).into();
    let (Some(r), Some(s)) = (r, s) else {
        return Err(CryptoError::InvalidSignature);
    };
    if r.is_zero().into() || s.is_zero().into() {
        return Err(CryptoError::InvalidSignature);
    }

    // Decompress R from r and the recovery-id parity bit.
    // recid >= 2 (R.x = r + n) has ~2^-128 probability and never occurs for the
    // precompile; we don't handle it (decompression simply fails), matching the
    // trait default.
    let y_is_odd = (recid & 1) != 0;
    let r_point: Option<AffinePoint> =
        AffinePoint::decompress(r_bytes, u8::from(y_is_odd).into()).into();
    let Some(r_point) = r_point else {
        return Err(CryptoError::RecoveryFailed);
    };
    let r_proj = ProjectivePoint::from(r_point);

    let z = <Scalar as Reduce<U256>>::reduce_bytes(&FieldBytes::from(*msg));
    let r_inv: Option<Scalar> = r.invert_vartime().into();
    let Some(r_inv) = r_inv else {
        return Err(CryptoError::RecoveryFailed);
    };
    let u1 = -(r_inv * z);
    let u2 = r_inv * s;

    // pk = u1·G + u2·R, accelerated via ECSM with a software fallback.
    // The ECSM path takes affine inputs and returns the affine result directly:
    // its inputs (G, R) and output are Z=1 points, so passing affines avoids the
    // wasteful projective→affine inversions (`to_affine` of a Z=1 point still runs
    // a full constant-time field inversion in k256). The rare software fallback
    // still converts via `to_affine`.
    let g = ProjectivePoint::GENERATOR;
    let pk_affine = ecsm_lincomb2(&AffinePoint::GENERATOR, &u1, &r_point, &u2)
        .unwrap_or_else(|| ProjectivePoint::lincomb(&g, &u1, &r_proj, &u2).to_affine());
    if bool::from(pk_affine.is_identity()) {
        return Err(CryptoError::RecoveryFailed);
    }

    // SEC1 uncompressed: 0x04 || X(32) || Y(32). Return X‖Y for the caller to hash.
    let uncompressed = pk_affine.to_encoded_point(false);
    let mut pk_bytes = [0u8; 64];
    pk_bytes.copy_from_slice(&uncompressed.as_bytes()[1..65]);
    Ok(pk_bytes)
}

/// ECSM-accelerated 2-term linear combination `k1·P1 + k2·P2`.
///
/// On riscv64 this reconstructs the full affine result from four x-only ECSM
/// queries (see [`lincomb2_with_oracle`]); on other targets, and whenever a
/// guard trips (degenerate input or oracle inconsistency), it returns `None`
/// so the caller uses the pure-Rust `ProjectivePoint::lincomb`.
#[cfg(target_arch = "riscv64")]
fn ecsm_lincomb2(
    a1: &AffinePoint,
    k1: &Scalar,
    a2: &AffinePoint,
    k2: &Scalar,
) -> Option<AffinePoint> {
    lincomb2_with_oracle(a1, k1, a2, k2, ecsm_oracle)
}

#[cfg(not(target_arch = "riscv64"))]
fn ecsm_lincomb2(
    _a1: &AffinePoint,
    _k1: &Scalar,
    _a2: &AffinePoint,
    _k2: &Scalar,
) -> Option<AffinePoint> {
    None
}

/// x-only scalar-mul oracle backed by the ECSM precompile: computes `x(k·P)`
/// for the curve point P whose x-coordinate is passed in. `x` must be the
/// x-coordinate of a curve point and `k` in `(0, N)` (N = curve order) —
/// guaranteed by the guards in [`lincomb2_with_oracle`]. Values cross the ABI
/// as 32-byte little-endian; `x_le` and `k_le` are distinct stack arrays so
/// the executor's `|addr_x_le − addr_k_le| ≥ 32` assumption holds by
/// construction.
#[cfg(target_arch = "riscv64")]
fn ecsm_oracle(x: &FieldElement, k: &Scalar) -> Option<FieldElement> {
    let x_be = x.to_bytes();
    let k_be = k.to_bytes();
    let mut x_le = [0u8; 32];
    let mut k_le = [0u8; 32];
    for i in 0..32 {
        x_le[i] = x_be[31 - i];
        k_le[i] = k_be[31 - i];
    }
    let mut xr_le = [0u8; 32];
    lambda_vm_syscalls::syscalls::ecsm_mul(&mut xr_le, &x_le, &k_le);
    xr_le.reverse();
    Option::from(FieldElement::from_bytes(&xr_le.into()))
}

/// Computes `k1·P1 + k2·P2` from four x-only oracle queries, or `None` if any
/// degenerate-configuration guard trips.
///
/// The lambda-vm ECSM precompile returns only `x(k·P)`. For `A = k1·P1` with
/// `P1 = (xp, yp)` fully known, query `xa = x(k1·P1)` and `xc = x((k1+1)·P1)`.
/// The chord-addition law gives `λ² = xc + xa + xp =: t` and `ya = yp + λ·dx`
/// with `dx = xa − xp`; substituting into `ya² = xa³ + b` makes λ *linear*:
/// `λ = (xa³ − xp³ − t·dx²) / (2·yp·dx)`. The wrong sign `−ya` would force
/// `x((k1−1)·P1) = xc`, i.e. `k1 ≡ 0` or `2·k1 ≡ 0 (mod n)`, excluded by the
/// scalar guards. x-only queries are parity-invariant (`x(k·P) = x(k·(−P))`),
/// so the precompile's canonical-y lift never matters. Same for `B = k2·P2`,
/// then `Q = A + B` is one affine addition. All three inversions are batched.
///
/// Generic over the oracle so unit tests can substitute a software stand-in.
#[cfg(any(target_arch = "riscv64", test))]
fn lincomb2_with_oracle<O>(
    a1: &AffinePoint,
    k1: &Scalar,
    a2: &AffinePoint,
    k2: &Scalar,
    oracle: O,
) -> Option<AffinePoint>
where
    O: Fn(&FieldElement, &Scalar) -> Option<FieldElement>,
{
    // Inputs are affine already (the ecrecover path lifts them from known Z=1
    // points), so no projective→affine inversion is needed here.
    if bool::from(a1.is_identity()) || bool::from(a2.is_identity()) {
        return None;
    }
    if scalar_near_edge(k1) || scalar_near_edge(k2) {
        return None;
    }

    let (x1, y1) = affine_xy(a1)?;
    let (x2, y2) = affine_xy(a2)?;

    let xa = oracle(&x1, k1)?;
    let xc1 = oracle(&x1, &(*k1 + Scalar::ONE))?;
    let xb = oracle(&x2, k2)?;
    let xc2 = oracle(&x2, &(*k2 + Scalar::ONE))?;

    let dx1 = (xa - x1).normalize();
    let dx2 = (xb - x2).normalize();
    let dxq = (xb - xa).normalize();
    if bool::from(dx1.is_zero()) || bool::from(dx2.is_zero()) || bool::from(dxq.is_zero()) {
        return None;
    }

    // One shared inversion for the two λ denominators and the final chord.
    let den1 = y1.double() * dx1;
    let den2 = y2.double() * dx2;
    let inv = Option::<FieldElement>::from((den1 * den2 * dxq).invert())?;
    let inv_den1 = inv * den2 * dxq;
    let inv_den2 = inv * den1 * dxq;
    let inv_dxq = inv * den1 * den2;

    let ya = solve_y(&x1, &y1, &xa, &xc1, &dx1, &inv_den1)?;
    let yb = solve_y(&x2, &y2, &xb, &xc2, &dx2, &inv_den2)?;

    // Q = A + B, with A ≠ ±B ensured by dxq ≠ 0.
    let lq = (yb - ya) * inv_dxq;
    let xq = (lq.square() - xa - xb).normalize();
    let yq = (lq * (xa - xq) - ya).normalize();

    // `point_from_xy` checks the result is on the curve as a cheap backstop:
    // it rejects gross off-curve garbage and falls back to software, but
    // correctness rests on the algebra above — an on-curve-but-wrong point
    // would still pass this check.
    point_from_xy(&xq, &yq)
}

/// Recovers `y(k·P)` from `xa = x(k·P)` and `xc = x((k+1)·P)`.
/// Returns `None` if `xc` is inconsistent with the computed `lambda`
/// (oracle misbehavior); degeneracy guards are in [`lincomb2_with_oracle`].
#[cfg(any(target_arch = "riscv64", test))]
fn solve_y(
    xp: &FieldElement,
    yp: &FieldElement,
    xa: &FieldElement,
    xc: &FieldElement,
    dx: &FieldElement,
    inv_den: &FieldElement,
) -> Option<FieldElement> {
    let t = *xc + xa + xp;
    let xa3 = xa.square() * xa;
    let xp3 = xp.square() * xp;
    let lambda = (xa3 - xp3 - t * dx.square()) * inv_den;
    if lambda.square().normalize() != t.normalize() {
        return None;
    }
    Some((*yp + lambda * dx).normalize())
}

/// `k ∈ {0, 1, n−1}`: fast early-exit before oracle calls.
/// k=0: invalid ecall scalar. k=1: dx=0. k=n-1: k+1 wraps to 0 mod n.
#[cfg(any(target_arch = "riscv64", test))]
fn scalar_near_edge(k: &Scalar) -> bool {
    use k256::elliptic_curve::subtle::ConstantTimeEq;
    bool::from(k.is_zero())
        || bool::from(k.ct_eq(&Scalar::ONE))
        || bool::from(k.ct_eq(&(-Scalar::ONE)))
}

/// Affine `(x, y)` of a non-identity point as field elements, via its SEC1
/// uncompressed encoding (k256 keeps `AffinePoint`'s coordinate fields private).
#[cfg(any(target_arch = "riscv64", test))]
fn affine_xy(p: &AffinePoint) -> Option<(FieldElement, FieldElement)> {
    let ep = p.to_encoded_point(false);
    let x = Option::<FieldElement>::from(FieldElement::from_bytes(ep.x()?))?;
    let y = Option::<FieldElement>::from(FieldElement::from_bytes(ep.y()?))?;
    Some((x, y))
}

/// Builds an affine curve point from coordinates, returning `None` if the point
/// is not on the curve (`AffinePoint::from_encoded_point` validates this).
#[cfg(any(target_arch = "riscv64", test))]
fn point_from_xy(x: &FieldElement, y: &FieldElement) -> Option<AffinePoint> {
    let ep = EncodedPoint::from_affine_coordinates(&x.to_bytes(), &y.to_bytes(), false);
    Option::<AffinePoint>::from(AffinePoint::from_encoded_point(&ep))
}

// ── Keccak-256 over the keccak_permute precompile (riscv64 guest) ───────────

/// Keccak-256 sponge with an injected permutation function.
///
/// Keccak-f[1600], rate 1088 bits (136 bytes), capacity 512 bits.
/// Padding: `0x01 ... 0x80` (multi-rate, last bit set). The state is a
/// 25-element u64 array; bytes are absorbed into the state via little-endian
/// XOR (matching the standard Keccak byte-to-lane mapping).
///
/// Gated to `riscv64 | test` so the generic function is available to the host
/// unit tests without being dead code in the non-test host build.
#[cfg(any(target_arch = "riscv64", test))]
fn keccak256_with_permute<F: FnMut(&mut [u64; 25])>(input: &[u8], mut permute: F) -> [u8; 32] {
    const RATE: usize = 136;

    let mut state = [0u64; 25];
    let mut offset = 0;

    while input.len() - offset >= RATE {
        absorb_block(&mut state, &input[offset..offset + RATE]);
        permute(&mut state);
        offset += RATE;
    }

    // Final block with multi-rate padding.
    let mut last = [0u8; RATE];
    let remaining = input.len() - offset;
    last[..remaining].copy_from_slice(&input[offset..]);
    last[remaining] ^= 0x01;
    last[RATE - 1] ^= 0x80;
    absorb_block(&mut state, &last);
    permute(&mut state);

    // Squeeze the first 32 bytes (four lanes) as little-endian.
    let mut output = [0u8; 32];
    for (i, lane) in state.iter().take(4).enumerate() {
        output[i * 8..i * 8 + 8].copy_from_slice(&lane.to_le_bytes());
    }
    output
}

/// Keccak-256 via LambdaVM's `keccak_permute` syscall (riscv64 guest only).
#[cfg(target_arch = "riscv64")]
fn keccak256_via_lambdavm(input: &[u8]) -> [u8; 32] {
    keccak256_with_permute(input, |s| lambda_vm_syscalls::syscalls::keccak_permute(s))
}

/// XOR one rate-sized block of bytes into the state lanes (little-endian).
#[cfg(any(target_arch = "riscv64", test))]
fn absorb_block(state: &mut [u64; 25], block: &[u8]) {
    for (lane, chunk) in state.iter_mut().zip(block.chunks_exact(8)) {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(chunk);
        *lane ^= u64::from_le_bytes(buf);
    }
}

#[cfg(test)]
mod tests;
