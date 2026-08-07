//! [`ethrex_crypto::Crypto`] implementation using OpenVM guest libraries.

use alloc::{sync::Arc, vec, vec::Vec};

use bls12_381::hash_to_curve::MapToCurve;
use ethrex_crypto::{Crypto, CryptoError, NativeCrypto};
use openvm_curve_utils::SubgroupCheck;
use openvm_ecc_guest::{
    AffinePoint, Group,
    algebra::IntMod,
    weierstrass::{IntrinsicCurve, WeierstrassPoint},
};
use openvm_k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use openvm_keccak256::keccak256;
use openvm_kzg::{Bytes32, Bytes48, EnvKzgSettings, KzgProof};
use openvm_p256::ecdsa::{
    Signature as P256Signature, VerifyingKey as P256VerifyingKey,
    signature::hazmat::PrehashVerifier,
};
use openvm_pairing::{
    PairingCheck,
    bls12_381::{self as bls, Bls12_381},
    bn254::{self as bn, Bn254},
};
use openvm_sha2::{Digest, Sha256};

// BN254 constants
const BN_FQ_LEN: usize = 32;
const BN_G1_LEN: usize = 64;
const BN_G2_LEN: usize = 128;
/// BN_SCALAR_LEN specifies the number of bytes needed to represent an Fr element.
/// This is an element in the scalar field of BN254.
const BN_SCALAR_LEN: usize = 32;

// BLS12-381 constants
const BLS_FP_LEN: usize = 48;
const BLS_G1_LEN: usize = 96;
const BLS_G2_LEN: usize = 192;

/// Returns a [`Crypto`] implementation backed by OpenVM guest libraries.
#[inline]
pub(super) fn crypto() -> Arc<dyn Crypto> {
    Arc::new(OpenVmCrypto)
}

#[derive(Debug, Default)]
struct OpenVmCrypto;

impl Crypto for OpenVmCrypto {
    #[inline]
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        mut recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        let mut signature =
            Signature::from_slice(sig).map_err(|_| CryptoError::InvalidSignature)?;

        if let Some(signature_normalized) = signature.normalize_s() {
            signature = signature_normalized;
            recid ^= 1;
        }

        let recovery_id = RecoveryId::from_byte(recid).ok_or(CryptoError::InvalidRecoveryId)?;

        let recovered_key =
            VerifyingKey::recover_from_prehash_noverify(msg, &signature.to_bytes(), recovery_id)
                .map_err(|_| CryptoError::RecoveryFailed)?;

        // Hash the uncompressed SEC1 key without the 0x04 prefix.
        let public_key = recovered_key.to_encoded_point(false);
        Ok(keccak256(&public_key.as_bytes()[1..]))
    }

    #[inline]
    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        keccak256(input)
    }

    #[inline]
    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        Sha256::digest(input).into()
    }

    #[inline]
    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        let p1 = read_bn_g1_point(p1)?;
        let p2 = read_bn_g1_point(p2)?;
        Ok(encode_bn_g1_point(p1 + p2))
    }

    #[inline]
    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        let point = read_bn_g1_point(point)?;
        let scalar = read_bn_scalar(scalar)?;
        Ok(encode_bn_g1_point(Bn254::msm(&[scalar], &[point])))
    }

    #[inline]
    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        if pairs.is_empty() {
            return Ok(true);
        }

        let mut g1_points = Vec::with_capacity(pairs.len());
        let mut g2_points = Vec::with_capacity(pairs.len());
        for (g1_bytes, g2_bytes) in pairs {
            let (g1_x, g1_y) = read_bn_g1_point(g1_bytes)?.into_coords();
            let (g2_x, g2_y) = read_bn_g2_point(g2_bytes)?.into_coords();
            g1_points.push(AffinePoint::new(g1_x, g1_y));
            g2_points.push(AffinePoint::new(g2_x, g2_y));
        }

        Ok(Bn254::pairing_check(&g1_points, &g2_points).is_ok())
    }

    #[inline]
    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if is_bn254_fr(modulus) {
            return Ok(accelerated_modexp_bn254_fr(base, exp));
        }
        NativeCrypto.modexp(base, exp, modulus)
    }

    #[inline]
    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        // `from_slice` rejects zero and non-canonical r/s scalars.
        let Ok(signature) = P256Signature::from_slice(sig) else {
            return false;
        };

        let x_bytes: &[u8; 32] = match pk[..32].try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };
        let y_bytes: &[u8; 32] = match pk[32..].try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };
        let encoded_point = openvm_p256::EncodedPoint::from_affine_coordinates(
            x_bytes.into(),
            y_bytes.into(),
            false,
        );
        let Ok(verifying_key) = P256VerifyingKey::from_encoded_point(&encoded_point) else {
            return false;
        };

        verifying_key.verify_prehash(msg, &signature).is_ok()
    }

    #[inline]
    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        let env = EnvKzgSettings::default();
        let kzg_settings = env.get();

        let commitment_bytes = Bytes48::from_slice(commitment)
            .map_err(|_| CryptoError::InvalidInput("invalid commitment bytes"))?;
        let z_bytes =
            Bytes32::from_slice(z).map_err(|_| CryptoError::InvalidInput("invalid z bytes"))?;
        let y_bytes =
            Bytes32::from_slice(y).map_err(|_| CryptoError::InvalidInput("invalid y bytes"))?;
        let proof_bytes = Bytes48::from_slice(proof)
            .map_err(|_| CryptoError::InvalidInput("invalid proof bytes"))?;

        let valid = KzgProof::verify_kzg_proof(
            &commitment_bytes,
            &z_bytes,
            &y_bytes,
            &proof_bytes,
            kzg_settings,
        )
        .map_err(|_| CryptoError::VerificationFailed)?;
        if valid {
            Ok(())
        } else {
            Err(CryptoError::VerificationFailed)
        }
    }

    #[inline]
    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        // EIP-2537 G1ADD validates on-curve only, not subgroup membership.
        let p1 = read_bls_g1_point_no_subgroup_check(&a)?;
        let p2 = read_bls_g1_point_no_subgroup_check(&b)?;
        Ok(encode_bls_g1_point(&(p1 + p2)))
    }

    #[inline]
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        let mut points = Vec::with_capacity(pairs.len());
        let mut scalars = Vec::with_capacity(pairs.len());
        for (point, scalar) in pairs {
            points.push(read_bls_g1_point(point)?);
            scalars.push(read_bls_scalar(scalar));
        }

        if points.is_empty() {
            return Ok([0u8; BLS_G1_LEN]);
        }

        Ok(encode_bls_g1_point(&Bls12_381::msm(&scalars, &points)))
    }

    #[inline]
    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        // EIP-2537 G2ADD validates on-curve only, not subgroup membership.
        let p1 = read_bls_g2_point_no_subgroup_check(&a)?;
        let p2 = read_bls_g2_point_no_subgroup_check(&b)?;
        Ok(encode_bls_g2_point(&(p1 + p2)))
    }

    #[inline]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        let mut points = Vec::with_capacity(pairs.len());
        let mut scalars = Vec::with_capacity(pairs.len());
        for (point, scalar) in pairs {
            points.push(read_bls_g2_point(point)?);
            scalars.push(read_bls_scalar(scalar));
        }

        if points.is_empty() {
            return Ok([0u8; BLS_G2_LEN]);
        }

        Ok(encode_bls_g2_point(&openvm_ecc_guest::msm(
            &scalars, &points,
        )))
    }

    #[inline]
    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        if pairs.is_empty() {
            return Ok(true);
        }

        let mut g1_points = Vec::with_capacity(pairs.len());
        let mut g2_points = Vec::with_capacity(pairs.len());
        for (g1_bytes, g2_bytes) in pairs {
            let (g1_x, g1_y) = read_bls_g1_point(g1_bytes)?.into_coords();
            let (g2_x, g2_y) = read_bls_g2_point(g2_bytes)?.into_coords();
            g1_points.push(AffinePoint::new(g1_x, g1_y));
            g2_points.push(AffinePoint::new(g2_x, g2_y));
        }

        Ok(Bls12_381::pairing_check(&g1_points, &g2_points).is_ok())
    }

    #[inline]
    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        type Fp = <bls12_381::G1Projective as MapToCurve>::Field;

        let fp_elem = Fp::from_bytes(fp)
            .into_option()
            .ok_or(CryptoError::InvalidInput("invalid Fp element"))?;

        let point = bls12_381::G1Projective::map_to_curve(&fp_elem).clear_h();
        serialize_bls12_g1(&bls12_381::G1Affine::from(point))
    }

    #[inline]
    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        type Fp = <bls12_381::G1Projective as MapToCurve>::Field;
        type Fp2 = <bls12_381::G2Projective as MapToCurve>::Field;

        let c0 = Fp::from_bytes(&fp2.0)
            .into_option()
            .ok_or(CryptoError::InvalidInput("invalid Fp2.c0 element"))?;
        let c1 = Fp::from_bytes(&fp2.1)
            .into_option()
            .ok_or(CryptoError::InvalidInput("invalid Fp2.c1 element"))?;

        let fp2_elem = Fp2 { c0, c1 };
        let point = bls12_381::G2Projective::map_to_curve(&fp2_elem).clear_h();
        serialize_bls12_g2(&bls12_381::G2Affine::from(point))
    }
}

/// Returns true if the modulus (big-endian, possibly with leading zeros) equals BN254 Fr.
fn is_bn254_fr(modulus: &[u8]) -> bool {
    // Strip leading zeros
    let stripped = match modulus.iter().position(|&b| b != 0) {
        Some(i) => &modulus[i..],
        None => return false, // all zeros
    };
    // bn::Scalar::MODULUS is little-endian; compare against reversed input
    stripped.len() == BN_SCALAR_LEN
        && stripped
            .iter()
            .rev()
            .eq(bn::Scalar::MODULUS.as_ref().iter())
}

/// Accelerated modexp for BN254 Fr using field arithmetic intrinsics.
fn accelerated_modexp_bn254_fr(base: &[u8], exp: &[u8]) -> Vec<u8> {
    use openvm_ecc_guest::algebra::{ExpBytes, Reduce};

    // OpenVM's field reduction requires inputs to be aligned to the field byte size.
    let padded_len = base
        .len()
        .next_multiple_of(BN_SCALAR_LEN)
        .max(BN_SCALAR_LEN);
    let mut padded = vec![0u8; padded_len];
    padded[padded_len - base.len()..].copy_from_slice(base);
    let base_fr = bn::Scalar::reduce_be_bytes(&padded);

    base_fr.exp_bytes(true, exp).to_be_bytes().as_ref().to_vec()
}

// Helper functions for BN254 operations

#[inline]
fn read_bn_fq(input: &[u8]) -> Result<bn::Fp, CryptoError> {
    if input.len() < BN_FQ_LEN {
        Err(CryptoError::InvalidInput("BN254 fp must be 32 bytes"))
    } else {
        bn::Fp::from_be_bytes(&input[..BN_FQ_LEN])
            .ok_or(CryptoError::InvalidInput("element not in BN254 base field"))
    }
}

#[inline]
fn read_bn_fq2(input: &[u8]) -> Result<bn::Fp2, CryptoError> {
    let y = read_bn_fq(&input[..BN_FQ_LEN])?;
    let x = read_bn_fq(&input[BN_FQ_LEN..BN_FQ_LEN * 2])?;
    Ok(bn::Fp2::new(x, y))
}

#[inline]
fn read_bn_g1_point(input: &[u8]) -> Result<bn::G1Affine, CryptoError> {
    if input.len() != BN_G1_LEN {
        return Err(CryptoError::InvalidInput("BN254 G1 point must be 64 bytes"));
    }
    let px = read_bn_fq(&input[0..BN_FQ_LEN])?;
    let py = read_bn_fq(&input[BN_FQ_LEN..BN_G1_LEN])?;
    // SAFETY: `read_bn_fq` produces canonical Fp elements; `from_xy` itself checks the curve
    // equation and returns `None` if `(px, py)` is not on the curve.
    let point = unsafe { bn::G1Affine::from_xy(px, py) }
        .ok_or(CryptoError::InvalidPoint("BN254 G1 point not on curve"))?;
    if point.is_in_correct_subgroup() {
        Ok(point)
    } else {
        Err(CryptoError::InvalidPoint("BN254 G1 point not in subgroup"))
    }
}

#[inline]
fn read_bn_g2_point(input: &[u8]) -> Result<bn::G2Affine, CryptoError> {
    if input.len() != BN_G2_LEN {
        return Err(CryptoError::InvalidInput(
            "BN254 G2 point must be 128 bytes",
        ));
    }
    let c0 = read_bn_fq2(&input[0..BN_G1_LEN])?;
    let c1 = read_bn_fq2(&input[BN_G1_LEN..BN_G2_LEN])?;
    // SAFETY: `read_bn_fq2` produces canonical Fp2 elements; `from_xy` itself checks the curve
    // equation and returns `None` if `(c0, c1)` is not on the twist.
    let point = unsafe { bn::G2Affine::from_xy(c0, c1) }
        .ok_or(CryptoError::InvalidPoint("BN254 G2 point not on curve"))?;
    if point.is_in_correct_subgroup() {
        Ok(point)
    } else {
        Err(CryptoError::InvalidPoint("BN254 G2 point not in subgroup"))
    }
}

#[inline]
fn encode_bn_g1_point(point: bn::G1Affine) -> [u8; BN_G1_LEN] {
    let mut output = [0u8; BN_G1_LEN];

    let x_bytes: &[u8] = point.x().as_le_bytes();
    let y_bytes: &[u8] = point.y().as_le_bytes();
    for i in 0..BN_FQ_LEN {
        output[i] = x_bytes[BN_FQ_LEN - 1 - i];
        output[i + BN_FQ_LEN] = y_bytes[BN_FQ_LEN - 1 - i];
    }
    output
}

/// Reads a scalar from the input slice. The scalar does not need to be canonical.
#[inline]
fn read_bn_scalar(input: &[u8]) -> Result<bn::Scalar, CryptoError> {
    if input.len() != BN_SCALAR_LEN {
        return Err(CryptoError::InvalidInput("BN254 scalar must be 32 bytes"));
    }
    Ok(bn::Scalar::from_be_bytes_unchecked(input))
}

// Helper functions for BLS12-381 operations

#[inline]
fn read_bls_fp(input: &[u8; 48]) -> Result<bls::Fp, CryptoError> {
    bls::Fp::from_be_bytes(input).ok_or(CryptoError::InvalidInput(
        "element not in BLS12-381 base field",
    ))
}

#[inline]
fn read_bls_fp2(c0: &[u8; 48], c1: &[u8; 48]) -> Result<bls::Fp2, CryptoError> {
    let real = read_bls_fp(c0)?;
    let imag = read_bls_fp(c1)?;
    Ok(bls::Fp2::new(real, imag))
}

#[inline]
fn read_bls_g1_point_no_subgroup_check(
    point: &([u8; 48], [u8; 48]),
) -> Result<bls::G1Affine, CryptoError> {
    let px = read_bls_fp(&point.0)?;
    let py = read_bls_fp(&point.1)?;
    // SAFETY: `read_bls_fp` produces canonical Fp elements; `from_xy` itself checks the curve
    // equation and returns `None` if `(px, py)` is not on the curve.
    unsafe { bls::G1Affine::from_xy(px, py) }
        .ok_or(CryptoError::InvalidPoint("BLS12-381 G1 point not on curve"))
}

#[inline]
fn read_bls_g1_point(point: &([u8; 48], [u8; 48])) -> Result<bls::G1Affine, CryptoError> {
    let point = read_bls_g1_point_no_subgroup_check(point)?;
    if point.is_in_correct_subgroup() {
        Ok(point)
    } else {
        Err(CryptoError::InvalidPoint(
            "BLS12-381 G1 point not in subgroup",
        ))
    }
}

#[inline]
fn read_bls_g2_point_no_subgroup_check(
    point: &([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
) -> Result<bls::G2Affine, CryptoError> {
    let x = read_bls_fp2(&point.0, &point.1)?;
    let y = read_bls_fp2(&point.2, &point.3)?;
    // SAFETY: `read_bls_fp2` produces canonical Fp2 elements; `from_xy` itself checks the curve
    // equation and returns `None` if `(x, y)` is not on the twist.
    unsafe { bls::G2Affine::from_xy(x, y) }
        .ok_or(CryptoError::InvalidPoint("BLS12-381 G2 point not on curve"))
}

#[inline]
fn read_bls_g2_point(
    point: &([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
) -> Result<bls::G2Affine, CryptoError> {
    let point = read_bls_g2_point_no_subgroup_check(point)?;
    if point.is_in_correct_subgroup() {
        Ok(point)
    } else {
        Err(CryptoError::InvalidPoint(
            "BLS12-381 G2 point not in subgroup",
        ))
    }
}

/// Reads a scalar from the input bytes. The scalar does not need to be canonical.
#[inline]
fn read_bls_scalar(input: &[u8; 32]) -> bls::Scalar {
    bls::Scalar::from_be_bytes_unchecked(input)
}

#[inline]
fn encode_bls_g1_point(point: &bls::G1Affine) -> [u8; BLS_G1_LEN] {
    if point.is_identity() {
        return [0u8; BLS_G1_LEN];
    }

    let mut output = [0u8; BLS_G1_LEN];
    let x_bytes: &[u8] = point.x().as_le_bytes();
    let y_bytes: &[u8] = point.y().as_le_bytes();
    for i in 0..BLS_FP_LEN {
        output[i] = x_bytes[BLS_FP_LEN - 1 - i];
        output[i + BLS_FP_LEN] = y_bytes[BLS_FP_LEN - 1 - i];
    }
    output
}

#[inline]
fn encode_bls_g2_point(point: &bls::G2Affine) -> [u8; BLS_G2_LEN] {
    if point.is_identity() {
        return [0u8; BLS_G2_LEN];
    }

    let mut output = [0u8; BLS_G2_LEN];
    let x = point.x();
    let y = point.y();
    let x_c0 = x.c0.as_le_bytes();
    let x_c1 = x.c1.as_le_bytes();
    let y_c0 = y.c0.as_le_bytes();
    let y_c1 = y.c1.as_le_bytes();
    for i in 0..BLS_FP_LEN {
        output[i] = x_c0[BLS_FP_LEN - 1 - i];
        output[i + BLS_FP_LEN] = x_c1[BLS_FP_LEN - 1 - i];
        output[i + (2 * BLS_FP_LEN)] = y_c0[BLS_FP_LEN - 1 - i];
        output[i + (3 * BLS_FP_LEN)] = y_c1[BLS_FP_LEN - 1 - i];
    }
    output
}

/// Serialize a BLS12-381 G1Affine point to 96 unpadded bytes (x || y, each 48 bytes).
fn serialize_bls12_g1(point: &bls12_381::G1Affine) -> Result<[u8; 96], CryptoError> {
    if bool::from(point.is_identity()) {
        return Ok([0u8; 96]);
    }

    Ok(point.to_uncompressed())
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
