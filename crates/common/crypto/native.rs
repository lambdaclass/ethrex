use ethereum_types::Address;
use sha2::Digest as _;

use crate::provider::{Crypto, CryptoError};

/// Native crypto implementation using system libraries.
#[derive(Debug)]
pub struct NativeCrypto;

impl Crypto for NativeCrypto {
    // ── ECDSA (secp256k1) ──────────────────────────────────────────────

    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        let recovery_id = secp256k1::ecdsa::RecoveryId::try_from(recid as i32)
            .map_err(|_| CryptoError::InvalidRecoveryId)?;

        let recoverable_sig =
            secp256k1::ecdsa::RecoverableSignature::from_compact(sig, recovery_id)
                .map_err(|_| CryptoError::InvalidSignature)?;

        let message = secp256k1::Message::from_digest(*msg);

        let public_key = recoverable_sig
            .recover(&message)
            .map_err(|_| CryptoError::RecoveryFailed)?;

        let hash =
            crate::keccak::keccak_hash(&public_key.serialize_uncompressed()[1..]);
        Ok(hash)
    }

    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
        // EIP-2: reject high-s signatures (s > secp256k1n/2)
        const SECP256K1_N_HALF: [u8; 32] =
            hex_literal::hex!("7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0");
        if sig[32..64] > SECP256K1_N_HALF[..] {
            return Err(CryptoError::InvalidSignature);
        }

        let recid_byte = sig[64] as i32;
        let recovery_id = secp256k1::ecdsa::RecoveryId::try_from(recid_byte)
            .map_err(|_| CryptoError::InvalidRecoveryId)?;

        let recoverable_sig =
            secp256k1::ecdsa::RecoverableSignature::from_compact(&sig[..64], recovery_id)
                .map_err(|_| CryptoError::InvalidSignature)?;

        let message = secp256k1::Message::from_digest(*msg);

        let public_key = secp256k1::SECP256K1
            .recover_ecdsa(&message, &recoverable_sig)
            .map_err(|_| CryptoError::RecoveryFailed)?;

        let hash =
            crate::keccak::keccak_hash(&public_key.serialize_uncompressed()[1..]);
        Ok(Address::from_slice(&hash[12..]))
    }

    // ── Hashing ────────────────────────────────────────────────────────

    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        sha2::Sha256::digest(input).into()
    }

    fn ripemd160(&self, input: &[u8]) -> [u8; 32] {
        let mut hasher = ripemd::Ripemd160::new();
        hasher.update(input);
        let result = hasher.finalize();

        let mut output = [0u8; 32];
        output[12..].copy_from_slice(&result);
        output
    }

    // ── BN254 (alt_bn128) ──────────────────────────────────────────────

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        use ark_bn254::Fq;
        use ark_ec::CurveGroup;
        use ark_ff::{BigInteger, PrimeField as _, Zero};

        let parse_point =
            |bytes: &[u8]| -> Result<ark_bn254::G1Affine, CryptoError> {
                if bytes.len() < 64 {
                    return Err(CryptoError::InvalidInput("G1 point must be 64 bytes"));
                }
                let x = Fq::from_be_bytes_mod_order(&bytes[..32]);
                let y = Fq::from_be_bytes_mod_order(&bytes[32..64]);

                if x.is_zero() && y.is_zero() {
                    return Ok(ark_bn254::G1Affine::identity());
                }

                let point = ark_bn254::G1Affine::new_unchecked(x, y);
                if !point.is_on_curve() {
                    return Err(CryptoError::InvalidPoint("G1 point not on curve"));
                }
                Ok(point)
            };

        let pt1 = parse_point(p1)?;
        let pt2 = parse_point(p2)?;

        #[allow(clippy::arithmetic_side_effects)]
        let sum = (pt1 + pt2).into_affine();

        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&sum.x.into_bigint().to_bytes_be());
        out[32..].copy_from_slice(&sum.y.into_bigint().to_bytes_be());
        Ok(out)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        use ark_bn254::{Fr as FrArk, Fq};
        use ark_ec::CurveGroup;
        use ark_ff::{BigInteger, PrimeField as _, Zero};
        use std::ops::Mul as _;

        if point.len() < 64 || scalar.len() < 32 {
            return Err(CryptoError::InvalidInput("invalid input length"));
        }

        let x = Fq::from_be_bytes_mod_order(&point[..32]);
        let y = Fq::from_be_bytes_mod_order(&point[32..64]);

        if x.is_zero() && y.is_zero() {
            return Ok([0u8; 64]);
        }

        let pt = ark_bn254::G1Affine::new_unchecked(x, y);
        if !pt.is_on_curve() {
            return Err(CryptoError::InvalidPoint("G1 point not on curve"));
        }

        let s = FrArk::from_be_bytes_mod_order(scalar);
        if s.is_zero() {
            return Ok([0u8; 64]);
        }

        let result = pt.mul(s).into_affine();

        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&result.x.into_bigint().to_bytes_be());
        out[32..].copy_from_slice(&result.y.into_bigint().to_bytes_be());
        Ok(out)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        use ark_bn254::{Bn254, G1Affine, G2Affine};
        use ark_ec::pairing::Pairing;
        use ark_ff::{Fp, One, QuadExtField, PrimeField as _};

        let mut g1_points = Vec::with_capacity(pairs.len());
        let mut g2_points = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs {
            // Parse G1: 64 bytes = x(32) || y(32), big-endian
            if g1_bytes.len() < 64 {
                return Err(CryptoError::InvalidInput("G1 must be 64 bytes"));
            }
            let g1x = Fp::from_le_bytes_mod_order(&{
                let mut b = g1_bytes[..32].to_vec();
                b.reverse();
                b
            });
            let g1y = Fp::from_le_bytes_mod_order(&{
                let mut b = g1_bytes[32..64].to_vec();
                b.reverse();
                b
            });

            let g1 = if g1x == ark_ff::Zero::zero() && g1y == ark_ff::Zero::zero() {
                G1Affine::identity()
            } else {
                let p = G1Affine::new_unchecked(g1x, g1y);
                if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
                    return Err(CryptoError::InvalidPoint("G1 not on BN254 curve"));
                }
                p
            };
            g1_points.push(g1);

            // Parse G2: 128 bytes = x_im(32) || x_re(32) || y_im(32) || y_re(32), big-endian
            // Note: EVM encodes G2 as (x_im, x_re, y_im, y_re)
            if g2_bytes.len() < 128 {
                return Err(CryptoError::InvalidInput("G2 must be 128 bytes"));
            }

            let g2_x_im = Fp::from_le_bytes_mod_order(&{
                let mut b = g2_bytes[..32].to_vec();
                b.reverse();
                b
            });
            let g2_x_re = Fp::from_le_bytes_mod_order(&{
                let mut b = g2_bytes[32..64].to_vec();
                b.reverse();
                b
            });
            let g2_y_im = Fp::from_le_bytes_mod_order(&{
                let mut b = g2_bytes[64..96].to_vec();
                b.reverse();
                b
            });
            let g2_y_re = Fp::from_le_bytes_mod_order(&{
                let mut b = g2_bytes[96..128].to_vec();
                b.reverse();
                b
            });

            let g2 = if g2_x_im == ark_ff::Zero::zero()
                && g2_x_re == ark_ff::Zero::zero()
                && g2_y_im == ark_ff::Zero::zero()
                && g2_y_re == ark_ff::Zero::zero()
            {
                G2Affine::identity()
            } else {
                let p = G2Affine::new_unchecked(
                    QuadExtField::new(g2_x_re, g2_x_im),
                    QuadExtField::new(g2_y_re, g2_y_im),
                );
                if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
                    return Err(CryptoError::InvalidPoint("G2 not on BN254 curve"));
                }
                p
            };
            g2_points.push(g2);
        }

        Ok(Bn254::multi_pairing(g1_points, g2_points).0 == QuadExtField::one())
    }

    // ── Modular arithmetic ─────────────────────────────────────────────

    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
        use malachite::base::num::arithmetic::traits::ModPow as _;
        use malachite::base::num::basic::traits::Zero as _;
        use malachite::{Natural, base::num::conversion::traits::*};

        let base_nat = Natural::from_power_of_2_digits_desc(8u64, base.iter().cloned())
            .ok_or(CryptoError::InvalidInput("base"))?;
        let exp_nat = Natural::from_power_of_2_digits_desc(8u64, exp.iter().cloned())
            .ok_or(CryptoError::InvalidInput("exponent"))?;
        let mod_nat = Natural::from_power_of_2_digits_desc(8u64, modulus.iter().cloned())
            .ok_or(CryptoError::InvalidInput("modulus"))?;

        let result = if mod_nat == Natural::ZERO {
            Natural::ZERO
        } else if exp_nat == Natural::ZERO {
            Natural::from(1_u8) % &mod_nat
        } else {
            let base_mod = base_nat % &mod_nat;
            base_mod.mod_pow(&exp_nat, &mod_nat)
        };

        let modulus_len = modulus.len();
        let res_bytes: Vec<u8> = result.to_power_of_2_digits_desc(8);

        // left-pad with zeros to match modulus length
        let mut out = vec![0u8; modulus_len];
        if res_bytes.len() <= modulus_len {
            let offset = modulus_len - res_bytes.len();
            out[offset..].copy_from_slice(&res_bytes);
        } else {
            // Shouldn't happen after mod_pow, but handle gracefully
            out.copy_from_slice(&res_bytes[res_bytes.len() - modulus_len..]);
        }
        Ok(out)
    }

    // ── Blake2 ─────────────────────────────────────────────────────────

    fn blake2_compress(
        &self,
        rounds: u32,
        h: &mut [u64; 8],
        m: [u64; 16],
        t: [u64; 2],
        f: bool,
    ) {
        #[allow(clippy::as_conversions)]
        crate::blake2f::blake2b_f(rounds as usize, h, &m, &t, f);
    }

    // ── secp256r1 (P-256) ──────────────────────────────────────────────

    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        use p256::{
            EncodedPoint,
            ecdsa::{Signature as P256Signature, signature::hazmat::PrehashVerifier},
            elliptic_curve::bigint::U256 as P256Uint,
        };

        // Validate r and s are non-zero and in range
        let r = P256Uint::from_be_slice(&sig[..32]);
        let s = P256Uint::from_be_slice(&sig[32..]);

        // P-256 curve order N
        const P256_N: P256Uint = P256Uint::from_be_hex(
            "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
        );

        if r == P256Uint::ZERO || r >= P256_N || s == P256Uint::ZERO || s >= P256_N {
            return false;
        }

        let x_bytes: &[u8; 32] = match pk[..32].try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };
        let y_bytes: &[u8; 32] = match pk[32..].try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };

        let Ok(verifier) = p256::ecdsa::VerifyingKey::from_encoded_point(
            &EncodedPoint::from_affine_coordinates(x_bytes.into(), y_bytes.into(), false),
        ) else {
            return false;
        };

        let r_arr: [u8; 32] = sig[..32].try_into().unwrap_or([0u8; 32]);
        let s_arr: [u8; 32] = sig[32..].try_into().unwrap_or([0u8; 32]);

        let Ok(signature) = P256Signature::from_scalars(r_arr, s_arr) else {
            return false;
        };

        verifier.verify_prehash(msg, &signature).is_ok()
    }

    // ── KZG ────────────────────────────────────────────────────────────

    #[cfg(feature = "c-kzg")]
    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        let c_kzg_settings = c_kzg::ethereum_kzg_settings(crate::kzg::KZG_PRECOMPUTE);
        c_kzg_settings
            .verify_kzg_proof(
                &(*commitment).into(),
                &(*z).into(),
                &(*y).into(),
                &(*proof).into(),
            )
            .map_err(|e| CryptoError::Other(e.to_string()))
            .and_then(|valid| {
                if valid {
                    Ok(())
                } else {
                    Err(CryptoError::VerificationFailed)
                }
            })
    }

    #[cfg(not(feature = "c-kzg"))]
    fn verify_kzg_proof(
        &self,
        _z: &[u8; 32],
        _y: &[u8; 32],
        _commitment: &[u8; 48],
        _proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        Err(CryptoError::Other(
            "c-kzg feature not enabled".to_string(),
        ))
    }

    #[cfg(feature = "c-kzg")]
    fn verify_blob_kzg_proof(
        &self,
        blob: &[u8],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<bool, CryptoError> {
        use crate::kzg::BYTES_PER_BLOB;

        let blob_arr: [u8; BYTES_PER_BLOB] = blob
            .try_into()
            .map_err(|_| CryptoError::InvalidInput("blob must be 131072 bytes"))?;

        let c_kzg_settings = c_kzg::ethereum_kzg_settings(crate::kzg::KZG_PRECOMPUTE);
        c_kzg_settings
            .verify_blob_kzg_proof(
                &blob_arr.into(),
                &(*commitment).into(),
                &(*proof).into(),
            )
            .map_err(|e| CryptoError::Other(e.to_string()))
    }

    #[cfg(not(feature = "c-kzg"))]
    fn verify_blob_kzg_proof(
        &self,
        _blob: &[u8],
        _commitment: &[u8; 48],
        _proof: &[u8; 48],
    ) -> Result<bool, CryptoError> {
        Err(CryptoError::Other(
            "c-kzg feature not enabled".to_string(),
        ))
    }

    // ── BLS12-381 (Prague, EIP-2537) ───────────────────────────────────

    fn bls12_381_g1_add(
        &self,
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

    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        use bls12_381::{G1Affine, G1Projective};
        use ff::Field as _;

        let mut result = G1Projective::identity();

        for (point_bytes, scalar_bytes) in pairs {
            let point = parse_bls12_g1(*point_bytes)?;
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

    fn bls12_381_g2_add(
        &self,
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

    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        use bls12_381::{G2Affine, G2Projective};

        let mut result = G2Projective::identity();

        for (point_bytes, scalar_bytes) in pairs {
            let point = parse_bls12_g2(*point_bytes)?;
            let scalar = parse_bls12_scalar(scalar_bytes);

            if scalar != bls12_381::Scalar::zero() {
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

    fn bls12_381_pairing_check(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), ([u8; 48], [u8; 48], [u8; 48], [u8; 48]))],
    ) -> Result<bool, CryptoError> {
        use bls12_381::{G1Affine, G2Prepared, Gt, multi_miller_loop};

        let mut points: Vec<(G1Affine, G2Prepared)> = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs {
            let g1 = parse_bls12_g1(*g1_bytes)?;
            let g2 = parse_bls12_g2(*g2_bytes)?;
            points.push((g1, G2Prepared::from(g2)));
        }

        let refs: Vec<(&G1Affine, &G2Prepared)> =
            points.iter().map(|(g1, g2)| (g1, g2)).collect();

        let result: Gt = multi_miller_loop(&refs).final_exponentiation();
        Ok(result == Gt::identity())
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        use bls12_381::{Fp, G1Affine, G1Projective, hash_to_curve::MapToCurve};

        let fp_elem = Fp::from_bytes(fp)
            .into_option()
            .ok_or(CryptoError::InvalidInput("invalid Fp element"))?;

        let point = G1Projective::map_to_curve(&fp_elem).clear_h();
        serialize_bls12_g1(&G1Affine::from(point))
    }

    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
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
}

// ── BLS12-381 helpers ──────────────────────────────────────────────────────

/// Parse an unpadded BLS12-381 G1 point from two 48-byte field elements.
fn parse_bls12_g1(
    (x_bytes, y_bytes): ([u8; 48], [u8; 48]),
) -> Result<bls12_381::G1Affine, CryptoError> {
    use bls12_381::G1Affine;

    const ALL_ZERO: [u8; 48] = [0u8; 48];

    if x_bytes == ALL_ZERO && y_bytes == ALL_ZERO {
        return Ok(G1Affine::identity());
    }

    // bls12_381 expects uncompressed: x(48) || y(48)
    let mut g1_bytes = [0u8; 96];
    g1_bytes[..48].copy_from_slice(&x_bytes);
    g1_bytes[48..].copy_from_slice(&y_bytes);

    let affine = G1Affine::from_uncompressed(&g1_bytes)
        .into_option()
        .ok_or(CryptoError::InvalidPoint("invalid BLS12-381 G1 point"))?;

    Ok(affine)
}

/// Parse an unpadded BLS12-381 G2 point from four 48-byte field elements.
/// EIP-2537 encodes G2 as (x_0, x_1, y_0, y_1) where x = x_0 + x_1*u in Fp2.
/// bls12_381 crate serializes as x_1 || x_0 || y_1 || y_0.
fn parse_bls12_g2(
    (x0, x1, y0, y1): ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
) -> Result<bls12_381::G2Affine, CryptoError> {
    use bls12_381::G2Affine;

    const ALL_ZERO: [u8; 48] = [0u8; 48];

    if x0 == ALL_ZERO && x1 == ALL_ZERO && y0 == ALL_ZERO && y1 == ALL_ZERO {
        return Ok(G2Affine::identity());
    }

    // bls12_381 serializes G2 uncompressed as: x_1 || x_0 || y_1 || y_0
    let mut g2_bytes = [0u8; 192];
    g2_bytes[0..48].copy_from_slice(&x1);
    g2_bytes[48..96].copy_from_slice(&x0);
    g2_bytes[96..144].copy_from_slice(&y1);
    g2_bytes[144..192].copy_from_slice(&y0);

    let affine = G2Affine::from_uncompressed(&g2_bytes)
        .into_option()
        .ok_or(CryptoError::InvalidPoint("invalid BLS12-381 G2 point"))?;

    Ok(affine)
}

/// Parse a 32-byte big-endian scalar as a BLS12-381 Scalar.
fn parse_bls12_scalar(scalar_bytes: &[u8; 32]) -> bls12_381::Scalar {
    // bls12_381::Scalar::from_raw expects 4 u64 limbs in little-endian limb order
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

    // bls12_381 uncompressed: x_1(48) || x_0(48) || y_1(48) || y_0(48)
    let raw = point.to_uncompressed();
    let mut out = [0u8; 192];
    // EIP-2537: x_0 || x_1 || y_0 || y_1
    out[0..48].copy_from_slice(&raw[48..96]); // x_0
    out[48..96].copy_from_slice(&raw[0..48]); // x_1
    out[96..144].copy_from_slice(&raw[144..192]); // y_0
    out[144..192].copy_from_slice(&raw[96..144]); // y_1
    Ok(out)
}
