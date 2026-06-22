/// Shared crypto helper functions used by multiple zkVM providers.
///
/// These functions are implemented in pure Rust using crates that are
/// patched by each zkVM toolchain (k256, substrate-bn) to use their
/// respective circuit accelerators transparently via Cargo patches.
use ethereum_types::Address;
use ethrex_crypto::{CryptoError, keccak::keccak_hash};

// ── k256 ECDSA ───────────────────────────────────────────────────────────────

/// ECDSA public key recovery using k256 (pure Rust, RISC-V compatible).
/// Used by the ECRECOVER precompile (0x01).
pub(crate) fn k256_ecrecover(
    sig: &[u8; 64],
    recid: u8,
    msg: &[u8; 32],
) -> Result<[u8; 32], CryptoError> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let mut sig_obj = Signature::from_slice(sig).map_err(|_| CryptoError::InvalidSignature)?;

    let mut recid_byte = recid;
    // k256 enforces canonical low-S for recovery.
    // If S is high, normalize s := n - s and flip the recovery parity bit.
    if let Some(low_s) = sig_obj.normalize_s() {
        sig_obj = low_s;
        recid_byte ^= 1;
    }

    let recovery_id = RecoveryId::from_byte(recid_byte).ok_or(CryptoError::InvalidRecoveryId)?;

    let vk = VerifyingKey::recover_from_prehash(msg, &sig_obj, recovery_id)
        .map_err(|_| CryptoError::RecoveryFailed)?;

    // SEC1 uncompressed: 0x04 || X(32) || Y(32). We need keccak(X||Y).
    let uncompressed = vk.to_encoded_point(false);
    let uncompressed_bytes = uncompressed.as_bytes();
    #[allow(clippy::indexing_slicing)]
    Ok(keccak_hash(&uncompressed_bytes[1..65]))
}

/// Transaction sender recovery using k256 (pure Rust, RISC-V compatible).
/// Used by tx.sender() and EIP-7702 authority recovery.
pub(crate) fn k256_recover_signer(sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    // EIP-2: reject high-s signatures (s > secp256k1n/2)
    const SECP256K1_N_HALF: [u8; 32] = [
        0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b,
        0x20, 0xa0,
    ];
    #[allow(clippy::indexing_slicing)]
    if sig[32..64] > SECP256K1_N_HALF[..] {
        return Err(CryptoError::InvalidSignature);
    }

    #[allow(clippy::indexing_slicing)]
    let signature = Signature::from_slice(&sig[..64]).map_err(|_| CryptoError::InvalidSignature)?;

    #[allow(clippy::indexing_slicing)]
    let recovery_id = RecoveryId::from_byte(sig[64]).ok_or(CryptoError::InvalidRecoveryId)?;

    let vk = VerifyingKey::recover_from_prehash(msg, &signature, recovery_id)
        .map_err(|_| CryptoError::RecoveryFailed)?;

    let uncompressed = vk.to_encoded_point(false);
    let uncompressed_bytes = uncompressed.as_bytes();
    #[allow(clippy::indexing_slicing)]
    let hash = keccak_hash(&uncompressed_bytes[1..65]);

    #[allow(clippy::indexing_slicing)]
    Ok(Address::from_slice(&hash[12..]))
}

// ── substrate-bn BN254 ───────────────────────────────────────────────────────
//
// These functions require the substrate-bn crate, which is only available when
// sp1, risc0, or zisk is enabled. The openvm feature only provides k256 and
// does not need BN254 via substrate-bn.
#[cfg(any(feature = "sp1", feature = "risc0", feature = "zisk"))]
/// BN254 G1 point addition using substrate-bn (pure Rust, RISC-V compatible).
/// Used by ZisK, which historically used substrate-bn for ecadd rather than ark-bn254.
pub(crate) fn substrate_bn_g1_add(p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
    use substrate_bn::{AffineG1, Fq, G1, Group};

    if p1.len() < 64 {
        return Err(CryptoError::InvalidInput("P1 must be at least 64 bytes"));
    }
    if p2.len() < 64 {
        return Err(CryptoError::InvalidInput("P2 must be at least 64 bytes"));
    }

    #[allow(clippy::indexing_slicing)]
    let p1x = Fq::from_slice(&p1[..32]).map_err(|_| CryptoError::InvalidInput("invalid P1.x"))?;
    #[allow(clippy::indexing_slicing)]
    let p1y = Fq::from_slice(&p1[32..64]).map_err(|_| CryptoError::InvalidInput("invalid P1.y"))?;

    let g1_a: G1 = if p1x.is_zero() && p1y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(p1x, p1y)
            .map_err(|_| CryptoError::InvalidPoint("P1 not on BN254 curve"))?
            .into()
    };

    #[allow(clippy::indexing_slicing)]
    let p2x = Fq::from_slice(&p2[..32]).map_err(|_| CryptoError::InvalidInput("invalid P2.x"))?;
    #[allow(clippy::indexing_slicing)]
    let p2y = Fq::from_slice(&p2[32..64]).map_err(|_| CryptoError::InvalidInput("invalid P2.y"))?;

    let g1_b: G1 = if p2x.is_zero() && p2y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(p2x, p2y)
            .map_err(|_| CryptoError::InvalidPoint("P2 not on BN254 curve"))?
            .into()
    };

    #[allow(clippy::arithmetic_side_effects)]
    let result = g1_a + g1_b;

    let mut out = [0u8; 64];
    if let Some(affine) = AffineG1::from_jacobian(result) {
        #[allow(clippy::indexing_slicing)]
        affine.x().to_big_endian(&mut out[..32]);
        #[allow(clippy::indexing_slicing)]
        affine.y().to_big_endian(&mut out[32..]);
    }
    Ok(out)
}

#[cfg(any(feature = "sp1", feature = "risc0", feature = "zisk"))]
/// BN254 G1 scalar multiplication using substrate-bn (pure Rust, RISC-V compatible).
/// Used by SP1 and ZisK where substrate-bn is patched for circuit acceleration.
pub(crate) fn substrate_bn_g1_mul(point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
    use substrate_bn::{AffineG1, Fq, Fr, G1, Group};

    if point.len() < 64 || scalar.len() < 32 {
        return Err(CryptoError::InvalidInput("invalid input length"));
    }

    #[allow(clippy::indexing_slicing)]
    let x = Fq::from_slice(&point[..32]).map_err(|_| CryptoError::InvalidInput("invalid Fq x"))?;
    #[allow(clippy::indexing_slicing)]
    let y =
        Fq::from_slice(&point[32..64]).map_err(|_| CryptoError::InvalidInput("invalid Fq y"))?;

    if x.is_zero() && y.is_zero() {
        return Ok([0u8; 64]);
    }

    let g1: G1 = AffineG1::new(x, y)
        .map_err(|_| CryptoError::InvalidPoint("G1 not on BN254 curve"))?
        .into();

    #[allow(clippy::indexing_slicing)]
    let s = Fr::from_slice(&scalar[..32]).map_err(|_| CryptoError::InvalidInput("invalid Fr"))?;

    if s.is_zero() {
        return Ok([0u8; 64]);
    }

    #[allow(clippy::arithmetic_side_effects)]
    let result = g1 * s;

    let mut out = [0u8; 64];
    if let Some(affine) = AffineG1::from_jacobian(result) {
        #[allow(clippy::indexing_slicing)]
        affine.x().to_big_endian(&mut out[..32]);
        #[allow(clippy::indexing_slicing)]
        affine.y().to_big_endian(&mut out[32..]);
    }
    Ok(out)
}

#[cfg(any(feature = "sp1", feature = "risc0", feature = "zisk"))]
/// BN254 pairing check using substrate-bn (pure Rust, RISC-V compatible).
/// Used by SP1, RISC0, and ZisK where substrate-bn is patched for circuit acceleration.
pub(crate) fn substrate_bn_pairing_check(pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
    use substrate_bn::{AffineG1, AffineG2, Fq, Fq2, G1, G2, Group};

    if pairs.is_empty() {
        return Ok(true);
    }

    let mut valid_batch = Vec::with_capacity(pairs.len());

    for (g1_bytes, g2_bytes) in pairs {
        if g1_bytes.len() < 64 {
            return Err(CryptoError::InvalidInput("G1 must be 64 bytes"));
        }
        if g2_bytes.len() < 128 {
            return Err(CryptoError::InvalidInput("G2 must be 128 bytes"));
        }

        // Parse G1
        #[allow(clippy::indexing_slicing)]
        let g1x = Fq::from_slice(&g1_bytes[..32]).map_err(|_| CryptoError::InvalidInput("G1.x"))?;
        #[allow(clippy::indexing_slicing)]
        let g1y =
            Fq::from_slice(&g1_bytes[32..64]).map_err(|_| CryptoError::InvalidInput("G1.y"))?;

        let g1: G1 = if g1x.is_zero() && g1y.is_zero() {
            G1::zero()
        } else {
            AffineG1::new(g1x, g1y)
                .map_err(|_| CryptoError::InvalidPoint("G1 not on curve"))?
                .into()
        };

        // Parse G2 — EVM encodes as (x_im, x_re, y_im, y_re) each 32 bytes
        #[allow(clippy::indexing_slicing)]
        let g2_x_im =
            Fq::from_slice(&g2_bytes[..32]).map_err(|_| CryptoError::InvalidInput("G2.x_im"))?;
        #[allow(clippy::indexing_slicing)]
        let g2_x_re =
            Fq::from_slice(&g2_bytes[32..64]).map_err(|_| CryptoError::InvalidInput("G2.x_re"))?;
        #[allow(clippy::indexing_slicing)]
        let g2_y_im =
            Fq::from_slice(&g2_bytes[64..96]).map_err(|_| CryptoError::InvalidInput("G2.y_im"))?;
        #[allow(clippy::indexing_slicing)]
        let g2_y_re =
            Fq::from_slice(&g2_bytes[96..128]).map_err(|_| CryptoError::InvalidInput("G2.y_re"))?;

        let g2: G2 =
            if g2_x_im.is_zero() && g2_x_re.is_zero() && g2_y_im.is_zero() && g2_y_re.is_zero() {
                G2::zero()
            } else {
                AffineG2::new(Fq2::new(g2_x_re, g2_x_im), Fq2::new(g2_y_re, g2_y_im))
                    .map_err(|_| CryptoError::InvalidPoint("G2 not on curve"))?
                    .into()
            };

        if g1.is_zero() || g2.is_zero() {
            continue;
        }
        valid_batch.push((g1, g2));
    }

    let result = substrate_bn::pairing_batch(&valid_batch);
    Ok(result == substrate_bn::Gt::one())
}

// ── bls12_381 BLS12-381 / EIP-2537 ───────────────────────────────────────────
//
// Portable pure-Rust BLS12-381 used by the SP1/RISC0/OpenVM providers (Zisk
// uses FFI instead). Relocated from `ethrex-crypto`'s `Crypto` trait defaults so
// the published `ethrex-crypto` crate carries no `bls12_381` git dependency; on
// the host, blst is the canonical backend. The `bls12_381`/`ff` crates are
// patched per-zkVM toolchain for circuit acceleration via Cargo `[patch]`.

/// BLS12-381 G1 addition. Returns a 96-byte unpadded G1 point.
pub(crate) fn bls12_381_g1_add(
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

/// BLS12-381 G1 multi-scalar multiplication. Returns a 96-byte unpadded G1 point.
#[allow(clippy::type_complexity)]
pub(crate) fn bls12_381_g1_msm(
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

/// BLS12-381 G2 addition. Returns a 192-byte unpadded G2 point.
pub(crate) fn bls12_381_g2_add(
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

/// BLS12-381 G2 multi-scalar multiplication. Returns a 192-byte unpadded G2 point.
#[allow(clippy::type_complexity)]
pub(crate) fn bls12_381_g2_msm(
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
pub(crate) fn bls12_381_pairing_check(
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

/// BLS12-381 map field element to G1 point.
pub(crate) fn bls12_381_fp_to_g1(fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
    use bls12_381::{Fp, G1Affine, G1Projective, hash_to_curve::MapToCurve};

    let fp_elem = Fp::from_bytes(fp)
        .into_option()
        .ok_or(CryptoError::InvalidInput("invalid Fp element"))?;

    let point = G1Projective::map_to_curve(&fp_elem).clear_h();
    serialize_bls12_g1(&G1Affine::from(point))
}

/// BLS12-381 map field element pair to G2 point.
pub(crate) fn bls12_381_fp2_to_g2(fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
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
