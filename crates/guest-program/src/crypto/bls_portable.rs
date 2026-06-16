//! Portable pure-Rust BLS12-381 (EIP-2537) used by the SP1/RISC0/OpenVM guest
//! `Crypto` providers.
//!
//! Mirrors the BN254/secp256k1 `shared.rs` pattern: pure-Rust crates that each
//! zkVM toolchain patches for circuit acceleration via Cargo `[patch]` (here,
//! the `bls12_381` git fork that exposes the affine constructors EIP-2537
//! needs). This implementation was relocated verbatim from `ethrex-crypto`'s
//! `Crypto` trait defaults so the published `ethrex-crypto` crate carries no
//! git dependency on the fork; on the host, blst is the canonical backend.
//!
//! NOTE: `ethrex-guest-program` is a `std` crate (no `#![no_std]`), so `Vec`
//! comes from the std prelude — unlike `ethrex-crypto` (no_std) which imported
//! `alloc::vec::Vec`.

use ethrex_crypto::CryptoError;

/// G1 addition. Returns 96-byte unpadded G1 point.
pub(crate) fn g1_add(
    a: ([u8; 48], [u8; 48]),
    b: ([u8; 48], [u8; 48]),
) -> Result<[u8; 96], CryptoError> {
    use bls12_381::{G1Affine, G1Projective};

    let pa = parse_bls12_g1(a)?;
    let pb = parse_bls12_g1(b)?;

    #[allow(clippy::arithmetic_side_effects)]
    let result = G1Affine::from(G1Projective::from(pa) + G1Projective::from(pb));
    serialize_bls12_g1(&result)
}

/// G1 multi-scalar multiplication. Returns 96-byte unpadded G1 point.
#[allow(clippy::type_complexity)]
pub(crate) fn g1_msm(
    pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
) -> Result<[u8; 96], CryptoError> {
    use bls12_381::{G1Affine, G1Projective};
    use ff::Field as _;

    let mut result = G1Projective::identity();

    for (point_bytes, scalar_bytes) in pairs {
        let point = parse_bls12_g1(*point_bytes)?;
        if !bool::from(point.is_torsion_free()) {
            return Err(CryptoError::InvalidPoint("G1 point not in subgroup"));
        }
        let scalar = parse_bls12_scalar(scalar_bytes);

        if !bool::from(scalar.is_zero()) {
            #[allow(clippy::arithmetic_side_effects)]
            let scaled: G1Projective = G1Projective::from(point) * scalar;
            #[allow(clippy::arithmetic_side_effects)]
            {
                result += scaled;
            }
        }
    }

    serialize_bls12_g1(&G1Affine::from(result))
}

/// G2 addition. Returns 192-byte unpadded G2 point.
pub(crate) fn g2_add(
    a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
) -> Result<[u8; 192], CryptoError> {
    use bls12_381::{G2Affine, G2Projective};

    let pa = parse_bls12_g2(a)?;
    let pb = parse_bls12_g2(b)?;

    #[allow(clippy::arithmetic_side_effects)]
    let result = G2Affine::from(G2Projective::from(pa) + G2Projective::from(pb));
    serialize_bls12_g2(&result)
}

/// G2 multi-scalar multiplication. Returns 192-byte unpadded G2 point.
#[allow(clippy::type_complexity)]
pub(crate) fn g2_msm(
    pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
) -> Result<[u8; 192], CryptoError> {
    use bls12_381::{G2Affine, G2Projective};
    use ff::Field as _;

    let mut result = G2Projective::identity();

    for (point_bytes, scalar_bytes) in pairs {
        let point = parse_bls12_g2(*point_bytes)?;
        if !bool::from(point.is_torsion_free()) {
            return Err(CryptoError::InvalidPoint("G2 point not in subgroup"));
        }
        let scalar = parse_bls12_scalar(scalar_bytes);

        if !bool::from(scalar.is_zero()) {
            #[allow(clippy::arithmetic_side_effects)]
            let scaled: G2Projective = G2Projective::from(point) * scalar;
            #[allow(clippy::arithmetic_side_effects)]
            {
                result += scaled;
            }
        }
    }

    serialize_bls12_g2(&G2Affine::from(result))
}

/// BLS12-381 pairing check.
#[allow(clippy::type_complexity)]
pub(crate) fn pairing_check(
    pairs: &[(
        ([u8; 48], [u8; 48]),
        ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    )],
) -> Result<bool, CryptoError> {
    use bls12_381::{G1Affine, G2Prepared, Gt, multi_miller_loop};

    let mut points: Vec<(G1Affine, G2Prepared)> = Vec::with_capacity(pairs.len());

    for (g1_bytes, g2_bytes) in pairs {
        let g1 = parse_bls12_g1(*g1_bytes)?;
        let g2 = parse_bls12_g2(*g2_bytes)?;
        // EIP-2537: pairing requires subgroup membership
        if !bool::from(g1.is_torsion_free()) {
            return Err(CryptoError::InvalidPoint("G1 not in subgroup"));
        }
        if !bool::from(g2.is_torsion_free()) {
            return Err(CryptoError::InvalidPoint("G2 not in subgroup"));
        }
        points.push((g1, G2Prepared::from(g2)));
    }

    let refs: Vec<(&G1Affine, &G2Prepared)> = points.iter().map(|(g1, g2)| (g1, g2)).collect();

    let result: Gt = multi_miller_loop(&refs).final_exponentiation();
    Ok(result == Gt::identity())
}

/// Map field element to G1 point.
pub(crate) fn fp_to_g1(fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
    use bls12_381::{Fp, G1Affine, G1Projective, hash_to_curve::MapToCurve};

    let fp_elem = Fp::from_bytes(fp)
        .into_option()
        .ok_or(CryptoError::InvalidInput("invalid Fp element"))?;

    let point = G1Projective::map_to_curve(&fp_elem).clear_h();
    serialize_bls12_g1(&G1Affine::from(point))
}

/// Map field element pair to G2 point.
pub(crate) fn fp2_to_g2(fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
    use bls12_381::{Fp, Fp2, G2Affine, G2Projective, hash_to_curve::MapToCurve};

    let c0 = Fp::from_bytes(&fp2.0)
        .into_option()
        .ok_or(CryptoError::InvalidInput("invalid Fp2.c0 element"))?;
    let c1 = Fp::from_bytes(&fp2.1)
        .into_option()
        .ok_or(CryptoError::InvalidInput("invalid Fp2.c1 element"))?;

    let fp2_elem = Fp2 { c0, c1 };
    let point = G2Projective::map_to_curve(&fp2_elem).clear_h();
    serialize_bls12_g2(&G2Affine::from(point))
}

// ── helpers (relocated verbatim from ethrex-crypto/provider.rs) ──────────────

/// Parse an unpadded BLS12-381 G1 point from two 48-byte field elements.
///
/// `Fp::from_bytes` validates that each coordinate is strictly less than the
/// field modulus, which also prevents the top bits from being misinterpreted
/// as BLS serialization flags.
fn parse_bls12_g1(
    (x_bytes, y_bytes): ([u8; 48], [u8; 48]),
) -> Result<bls12_381::G1Affine, CryptoError> {
    use bls12_381::{Fp, G1Affine};

    let x = Fp::from_bytes(&x_bytes)
        .into_option()
        .ok_or(CryptoError::InvalidInput(
            "G1 x coordinate >= field modulus",
        ))?;
    let y = Fp::from_bytes(&y_bytes)
        .into_option()
        .ok_or(CryptoError::InvalidInput(
            "G1 y coordinate >= field modulus",
        ))?;

    if x.is_zero().into() && y.is_zero().into() {
        return Ok(G1Affine::identity());
    }

    let affine = G1Affine::new_unchecked(x, y);

    if !bool::from(affine.is_on_curve()) {
        return Err(CryptoError::InvalidPoint("G1 point not on curve"));
    }

    Ok(affine)
}

/// Parse an unpadded BLS12-381 G2 point from four 48-byte field elements.
/// EIP-2537 encodes G2 as (x_0, x_1, y_0, y_1) where x = x_0 + x_1*u in Fp2.
fn parse_bls12_g2(
    (x0, x1, y0, y1): ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
) -> Result<bls12_381::G2Affine, CryptoError> {
    use bls12_381::{Fp, Fp2, G2Affine};

    let x0 = Fp::from_bytes(&x0)
        .into_option()
        .ok_or(CryptoError::InvalidInput("G2 x0 >= field modulus"))?;
    let x1 = Fp::from_bytes(&x1)
        .into_option()
        .ok_or(CryptoError::InvalidInput("G2 x1 >= field modulus"))?;
    let y0 = Fp::from_bytes(&y0)
        .into_option()
        .ok_or(CryptoError::InvalidInput("G2 y0 >= field modulus"))?;
    let y1 = Fp::from_bytes(&y1)
        .into_option()
        .ok_or(CryptoError::InvalidInput("G2 y1 >= field modulus"))?;

    if x0.is_zero().into() && x1.is_zero().into() && y0.is_zero().into() && y1.is_zero().into() {
        return Ok(G2Affine::identity());
    }

    let affine = G2Affine::new_unchecked(Fp2 { c0: x0, c1: x1 }, Fp2 { c0: y0, c1: y1 });

    if !bool::from(affine.is_on_curve()) {
        return Err(CryptoError::InvalidPoint("G2 point not on curve"));
    }

    Ok(affine)
}

/// Parse a 32-byte big-endian scalar as a BLS12-381 Scalar.
fn parse_bls12_scalar(scalar_bytes: &[u8; 32]) -> bls12_381::Scalar {
    let scalar_le = [
        u64::from_be_bytes(scalar_bytes[24..32].try_into().unwrap_or([0u8; 8])),
        u64::from_be_bytes(scalar_bytes[16..24].try_into().unwrap_or([0u8; 8])),
        u64::from_be_bytes(scalar_bytes[8..16].try_into().unwrap_or([0u8; 8])),
        u64::from_be_bytes(scalar_bytes[0..8].try_into().unwrap_or([0u8; 8])),
    ];
    bls12_381::Scalar::from_raw(scalar_le)
}

/// Serialize a BLS12-381 G1Affine point to 96 unpadded bytes (x || y, each 48 bytes).
fn serialize_bls12_g1(point: &bls12_381::G1Affine) -> Result<[u8; 96], CryptoError> {
    if bool::from(point.is_identity()) {
        return Ok([0u8; 96]);
    }

    let uncompressed = point.to_uncompressed();
    Ok(uncompressed)
}

/// Serialize a BLS12-381 G2Affine point to 192 unpadded bytes.
/// bls12_381 serializes as x_1 || x_0 || y_1 || y_0 (192 bytes).
/// We output as x_0 || x_1 || y_0 || y_1 to match EIP-2537 convention.
fn serialize_bls12_g2(point: &bls12_381::G2Affine) -> Result<[u8; 192], CryptoError> {
    if bool::from(point.is_identity()) {
        return Ok([0u8; 192]);
    }

    let raw = point.to_uncompressed();
    let mut out = [0u8; 192];
    out[0..48].copy_from_slice(&raw[48..96]); // x_0
    out[48..96].copy_from_slice(&raw[0..48]); // x_1
    out[96..144].copy_from_slice(&raw[144..192]); // y_0
    out[144..192].copy_from_slice(&raw[96..144]); // y_1
    Ok(out)
}

/// Emit the 7 BLS12-381 `Crypto` trait methods for a guest provider, delegating
/// to the portable implementation above. Invoke inside `impl Crypto for X { … }`
/// for the SP1/RISC0/OpenVM providers (Zisk overrides these via FFI instead).
macro_rules! impl_portable_bls12_381 {
    () => {
        fn bls12_381_g1_add(
            &self,
            a: ([u8; 48], [u8; 48]),
            b: ([u8; 48], [u8; 48]),
        ) -> Result<[u8; 96], ::ethrex_crypto::CryptoError> {
            $crate::crypto::bls_portable::g1_add(a, b)
        }

        #[allow(clippy::type_complexity)]
        fn bls12_381_g1_msm(
            &self,
            pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
        ) -> Result<[u8; 96], ::ethrex_crypto::CryptoError> {
            $crate::crypto::bls_portable::g1_msm(pairs)
        }

        fn bls12_381_g2_add(
            &self,
            a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
            b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        ) -> Result<[u8; 192], ::ethrex_crypto::CryptoError> {
            $crate::crypto::bls_portable::g2_add(a, b)
        }

        #[allow(clippy::type_complexity)]
        fn bls12_381_g2_msm(
            &self,
            pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
        ) -> Result<[u8; 192], ::ethrex_crypto::CryptoError> {
            $crate::crypto::bls_portable::g2_msm(pairs)
        }

        #[allow(clippy::type_complexity)]
        fn bls12_381_pairing_check(
            &self,
            pairs: &[(
                ([u8; 48], [u8; 48]),
                ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
            )],
        ) -> Result<bool, ::ethrex_crypto::CryptoError> {
            $crate::crypto::bls_portable::pairing_check(pairs)
        }

        fn bls12_381_fp_to_g1(
            &self,
            fp: &[u8; 48],
        ) -> Result<[u8; 96], ::ethrex_crypto::CryptoError> {
            $crate::crypto::bls_portable::fp_to_g1(fp)
        }

        fn bls12_381_fp2_to_g2(
            &self,
            fp2: ([u8; 48], [u8; 48]),
        ) -> Result<[u8; 192], ::ethrex_crypto::CryptoError> {
            $crate::crypto::bls_portable::fp2_to_g2(fp2)
        }
    };
}

pub(crate) use impl_portable_bls12_381;
