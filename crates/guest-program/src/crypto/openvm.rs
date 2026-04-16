use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::openvm_subgroup_check::SubgroupCheck;
use super::shared::{k256_ecrecover, k256_recover_signer};

/// OpenVM crypto provider.
///
/// Uses OpenVM guest-lib accelerators for keccak256, sha256, BN254, BLS12-381,
/// and P-256 operations. Falls back to k256 (patched by OpenVM) for secp256k1
/// ECDSA. Operations without an OpenVM extension (blake2, hash-to-curve,
/// etc.) use the trait defaults.
#[derive(Debug)]
pub struct OpenVmCrypto;

impl Crypto for OpenVmCrypto {
    // ── ECDSA (secp256k1) — via patched k256 ─────────────────────────────

    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        k256_ecrecover(sig, recid, msg)
    }

    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
        k256_recover_signer(sig, msg)
    }

    // ── Hashing ──────────────────────────────────────────────────────────

    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        openvm_keccak256::keccak256(input)
    }

    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        use openvm_sha2::Digest;
        openvm_sha2::Sha256::digest(input).into()
    }

    // ── BN254 (alt_bn128) ────────────────────────────────────────────────

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        use openvm_ecc_guest::Group;

        let a = decode_bn254_g1(p1)?;
        let b = decode_bn254_g1(p2)?;

        if a.is_identity() && b.is_identity() {
            return Ok([0u8; 64]);
        }
        if a.is_identity() {
            return encode_bn254_g1(&b);
        }
        if b.is_identity() {
            return encode_bn254_g1(&a);
        }

        #[allow(clippy::arithmetic_side_effects)]
        let result = a + &b;
        encode_bn254_g1(&result)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        use openvm_algebra_guest::IntMod;
        use openvm_ecc_guest::{weierstrass::IntrinsicCurve, Group};
        use openvm_pairing::bn254::{Bn254, Scalar as Bn254Scalar};

        if point.len() < 64 || scalar.len() < 32 {
            return Err(CryptoError::InvalidInput("invalid input length"));
        }

        let pt = decode_bn254_g1(point)?;
        if pt.is_identity() {
            return Ok([0u8; 64]);
        }

        let s = Bn254Scalar::from_be_bytes_unchecked(scalar);
        if s == Bn254Scalar::ZERO {
            return Ok([0u8; 64]);
        }

        let result = Bn254::msm(&[s], &[pt]);
        encode_bn254_g1(&result)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        use openvm_algebra_guest::IntMod;
        use openvm_ecc_guest::{weierstrass::WeierstrassPoint, AffinePoint};
        use openvm_pairing::bn254::{Bn254, Fp as Bn254Fp, Fp2 as Bn254Fp2};
        use openvm_pairing_guest::pairing::PairingCheck;

        if pairs.is_empty() {
            return Ok(true);
        }

        let mut g1_points: Vec<AffinePoint<Bn254Fp>> = Vec::with_capacity(pairs.len());
        let mut g2_points: Vec<AffinePoint<Bn254Fp2>> = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs {
            if g1_bytes.len() < 64 {
                return Err(CryptoError::InvalidInput("G1 must be 64 bytes"));
            }
            if g2_bytes.len() < 128 {
                return Err(CryptoError::InvalidInput("G2 must be 128 bytes"));
            }

            // Parse G1 — BN254 G1 has h=1, so from_xy (curve check) is sufficient
            let g1 = decode_bn254_g1(g1_bytes)?;
            let (g1_x, g1_y) = g1.into_coords();
            g1_points.push(AffinePoint::new(g1_x, g1_y));

            // Parse G2: EVM encodes as (x_im[32] || x_re[32] || y_im[32] || y_re[32])
            let g2_x_im = Bn254Fp::from_be_bytes(&g2_bytes[..32]);
            let g2_x_re = Bn254Fp::from_be_bytes(&g2_bytes[32..64]);
            let g2_y_im = Bn254Fp::from_be_bytes(&g2_bytes[64..96]);
            let g2_y_re = Bn254Fp::from_be_bytes(&g2_bytes[96..128]);
            let (Some(g2_x_im), Some(g2_x_re), Some(g2_y_im), Some(g2_y_re)) =
                (g2_x_im, g2_x_re, g2_y_im, g2_y_re)
            else {
                return Err(CryptoError::InvalidInput("G2 coordinate >= field modulus"));
            };

            // OpenVM Fp2 is (c0=real, c1=imaginary)
            let g2_x = Bn254Fp2::new(g2_x_re, g2_x_im);
            let g2_y = Bn254Fp2::new(g2_y_re, g2_y_im);

            // Check if point is zero (identity)
            if g2_x == Bn254Fp2::ZERO && g2_y == Bn254Fp2::ZERO {
                g2_points.push(AffinePoint::new(g2_x, g2_y));
                continue;
            }

            // Curve check via from_xy
            let g2_point = openvm_pairing::bn254::G2Affine::from_xy(g2_x.clone(), g2_y.clone())
                .ok_or(CryptoError::InvalidPoint("G2 point not on curve"))?;

            // Subgroup check — BN254 G2 has cofactor > 1
            if !g2_point.is_in_correct_subgroup() {
                return Err(CryptoError::InvalidPoint("G2 point not in subgroup"));
            }

            g2_points.push(AffinePoint::new(g2_x, g2_y));
        }

        match Bn254::pairing_check(&g1_points, &g2_points) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    // ── secp256r1 (P-256) ────────────────────────────────────────────────

    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        use openvm_p256::ecdsa::{signature::hazmat::PrehashVerifier, Signature, VerifyingKey};

        // Build SEC1 uncompressed key: 0x04 || x[32] || y[32]
        let mut sec1 = [0u8; 65];
        sec1[0] = 0x04;
        sec1[1..33].copy_from_slice(&pk[..32]);
        sec1[33..65].copy_from_slice(&pk[32..]);

        let Ok(verifier) = VerifyingKey::from_sec1_bytes(&sec1) else {
            return false;
        };

        let Ok(signature) = Signature::from_slice(sig) else {
            return false;
        };

        verifier.verify_prehash(msg, &signature).is_ok()
    }

    // ── BLS12-381 (EIP-2537) ─────────────────────────────────────────────

    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        use openvm_ecc_guest::Group;

        let pa = decode_bls12_g1(a)?;
        let pb = decode_bls12_g1(b)?;

        // Subgroup checks for BLS12-381 G1 (cofactor > 1)
        if !pa.is_identity() && !pa.is_in_correct_subgroup() {
            return Err(CryptoError::InvalidPoint("G1 point not in subgroup"));
        }
        if !pb.is_identity() && !pb.is_in_correct_subgroup() {
            return Err(CryptoError::InvalidPoint("G1 point not in subgroup"));
        }

        if pa.is_identity() && pb.is_identity() {
            return Ok([0u8; 96]);
        }
        if pa.is_identity() {
            return encode_bls12_g1(&pb);
        }
        if pb.is_identity() {
            return encode_bls12_g1(&pa);
        }

        #[allow(clippy::arithmetic_side_effects)]
        let result = pa + &pb;
        encode_bls12_g1(&result)
    }

    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        use openvm_algebra_guest::IntMod;
        use openvm_ecc_guest::{weierstrass::IntrinsicCurve, Group};
        use openvm_pairing::bls12_381::{Bls12_381, Scalar as Bls12Scalar};

        let mut bases = Vec::with_capacity(pairs.len());
        let mut scalars = Vec::with_capacity(pairs.len());

        for (point_bytes, scalar_bytes) in pairs {
            let point = decode_bls12_g1(*point_bytes)?;

            // Subgroup check for BLS12-381 G1 (cofactor > 1)
            if !point.is_identity() && !point.is_in_correct_subgroup() {
                return Err(CryptoError::InvalidPoint("G1 point not in subgroup"));
            }

            let scalar = Bls12Scalar::from_be_bytes_unchecked(scalar_bytes);

            if scalar == Bls12Scalar::ZERO || point.is_identity() {
                continue;
            }
            bases.push(point);
            scalars.push(scalar);
        }

        if bases.is_empty() {
            return Ok([0u8; 96]);
        }

        let result = Bls12_381::msm(&scalars, &bases);
        encode_bls12_g1(&result)
    }

    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        use openvm_ecc_guest::Group;

        let pa = decode_bls12_g2(a)?;
        let pb = decode_bls12_g2(b)?;

        if pa.is_identity() && pb.is_identity() {
            return Ok([0u8; 192]);
        }
        if pa.is_identity() {
            return encode_bls12_g2(&pb);
        }
        if pb.is_identity() {
            return encode_bls12_g2(&pa);
        }

        #[allow(clippy::arithmetic_side_effects)]
        let result = pa + &pb;
        encode_bls12_g2(&result)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        use openvm_algebra_guest::IntMod;
        use openvm_ecc_guest::Group;
        use openvm_pairing::bls12_381::Scalar as Bls12Scalar;

        let mut bases = Vec::with_capacity(pairs.len());
        let mut scalars = Vec::with_capacity(pairs.len());

        for (point_bytes, scalar_bytes) in pairs {
            let point = decode_bls12_g2(*point_bytes)?;
            let scalar = Bls12Scalar::from_be_bytes_unchecked(scalar_bytes);

            if scalar == Bls12Scalar::ZERO || point.is_identity() {
                continue;
            }
            bases.push(point);
            scalars.push(scalar);
        }

        if bases.is_empty() {
            return Ok([0u8; 192]);
        }

        let result = openvm_ecc_guest::msm(&scalars, &bases);
        encode_bls12_g2(&result)
    }

    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        use openvm_ecc_guest::{weierstrass::WeierstrassPoint, AffinePoint, Group};
        use openvm_pairing::bls12_381::{Bls12_381, Fp as Bls12Fp, Fp2 as Bls12Fp2};
        use openvm_pairing_guest::pairing::PairingCheck;

        if pairs.is_empty() {
            return Ok(true);
        }

        let mut g1_points: Vec<AffinePoint<Bls12Fp>> = Vec::with_capacity(pairs.len());
        let mut g2_points: Vec<AffinePoint<Bls12Fp2>> = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs {
            // Parse G1 with curve check and subgroup check
            let g1 = decode_bls12_g1(*g1_bytes)?;
            if !g1.is_identity() && !g1.is_in_correct_subgroup() {
                return Err(CryptoError::InvalidPoint("G1 not in subgroup"));
            }
            let (g1_x, g1_y) = g1.into_coords();
            g1_points.push(AffinePoint::new(g1_x, g1_y));

            // Parse G2 with curve check and subgroup check
            let g2 = decode_bls12_g2(*g2_bytes)?;
            if !g2.is_identity() && !g2.is_in_correct_subgroup() {
                return Err(CryptoError::InvalidPoint("G2 not in subgroup"));
            }
            let (g2_x, g2_y) = g2.into_coords();
            g2_points.push(AffinePoint::new(g2_x, g2_y));
        }

        match Bls12_381::pairing_check(&g1_points, &g2_points) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    // ── Modular arithmetic ───────────────────────────────────────────────

    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if is_bn254_fr(modulus) {
            return Ok(accelerated_modexp_bn254_fr(base, exp));
        }
        // Fall back to BigUint-based implementation (no-std compatible)
        modexp_fallback(base, exp, modulus)
    }
}

// ── BN254 helpers ─────────────────────────────────────────────────────────────

type Bn254G1 = openvm_pairing::bn254::G1Affine;

/// Decode a BN254 G1 point from 64 big-endian bytes (x[32] || y[32]).
/// Uses `from_xy` which validates the point is on the curve.
/// BN254 G1 has cofactor h=1, so curve check is sufficient for subgroup membership.
fn decode_bn254_g1(bytes: &[u8]) -> Result<Bn254G1, CryptoError> {
    use openvm_algebra_guest::IntMod;
    use openvm_ecc_guest::weierstrass::WeierstrassPoint;
    use openvm_pairing::bn254::Fp as Bn254Fp;

    if bytes.len() < 64 {
        return Err(CryptoError::InvalidInput("G1 point must be 64 bytes"));
    }

    let x = Bn254Fp::from_be_bytes(&bytes[..32]);
    let y = Bn254Fp::from_be_bytes(&bytes[32..64]);
    let (Some(x), Some(y)) = (x, y) else {
        return Err(CryptoError::InvalidInput("coordinate >= field modulus"));
    };

    // from_xy validates the curve equation (identity handled as (0,0))
    WeierstrassPoint::from_xy(x, y)
        .ok_or(CryptoError::InvalidPoint("G1 point not on curve"))
}

/// Encode a BN254 G1 point to 64 big-endian bytes.
fn encode_bn254_g1(point: &Bn254G1) -> Result<[u8; 64], CryptoError> {
    use openvm_algebra_guest::IntMod;
    use openvm_ecc_guest::{weierstrass::WeierstrassPoint, Group};

    if point.is_identity() {
        return Ok([0u8; 64]);
    }

    let x_be = point.x().to_be_bytes();
    let y_be = point.y().to_be_bytes();
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(x_be.as_ref());
    out[32..].copy_from_slice(y_be.as_ref());
    Ok(out)
}

// ── BLS12-381 helpers ─────────────────────────────────────────────────────────

type Bls12G1 = openvm_pairing::bls12_381::G1Affine;
type Bls12G2 = openvm_pairing::bls12_381::G2Affine;

/// Decode a BLS12-381 G1 point from two 48-byte big-endian field elements.
/// Uses `from_xy` which validates the point is on the curve.
fn decode_bls12_g1((x_bytes, y_bytes): ([u8; 48], [u8; 48])) -> Result<Bls12G1, CryptoError> {
    use openvm_algebra_guest::IntMod;
    use openvm_ecc_guest::weierstrass::WeierstrassPoint;
    use openvm_pairing::bls12_381::Fp as Bls12Fp;

    let x = Bls12Fp::from_be_bytes(&x_bytes);
    let y = Bls12Fp::from_be_bytes(&y_bytes);
    let (Some(x), Some(y)) = (x, y) else {
        return Err(CryptoError::InvalidInput(
            "G1 coordinate >= field modulus",
        ));
    };

    // from_xy validates the curve equation (identity handled as (0,0))
    WeierstrassPoint::from_xy(x, y)
        .ok_or(CryptoError::InvalidPoint("G1 point not on curve"))
}

/// Decode a BLS12-381 G2 point from four 48-byte big-endian field elements.
/// EIP-2537 encodes as (x_0[48] || x_1[48] || y_0[48] || y_1[48]) where Fp2 = x_0 + x_1*u.
fn decode_bls12_g2(
    (x0_bytes, x1_bytes, y0_bytes, y1_bytes): ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
) -> Result<Bls12G2, CryptoError> {
    use openvm_algebra_guest::IntMod;
    use openvm_ecc_guest::weierstrass::WeierstrassPoint;
    use openvm_pairing::bls12_381::{Fp as Bls12Fp, Fp2 as Bls12Fp2};

    let x0 = Bls12Fp::from_be_bytes(&x0_bytes);
    let x1 = Bls12Fp::from_be_bytes(&x1_bytes);
    let y0 = Bls12Fp::from_be_bytes(&y0_bytes);
    let y1 = Bls12Fp::from_be_bytes(&y1_bytes);
    let (Some(x0), Some(x1), Some(y0), Some(y1)) = (x0, x1, y0, y1) else {
        return Err(CryptoError::InvalidInput("G2 coordinate >= field modulus"));
    };

    let x = Bls12Fp2::new(x0, x1);
    let y = Bls12Fp2::new(y0, y1);

    // from_xy validates the curve equation (identity handled as (0,0))
    WeierstrassPoint::from_xy(x, y)
        .ok_or(CryptoError::InvalidPoint("G2 point not on curve"))
}

/// Encode a BLS12-381 G1 point to 96 big-endian bytes.
fn encode_bls12_g1(point: &Bls12G1) -> Result<[u8; 96], CryptoError> {
    use openvm_algebra_guest::IntMod;
    use openvm_ecc_guest::{weierstrass::WeierstrassPoint, Group};

    if point.is_identity() {
        return Ok([0u8; 96]);
    }

    let x_le = point.x().as_le_bytes();
    let y_le = point.y().as_le_bytes();
    let mut out = [0u8; 96];
    for i in 0..48 {
        out[i] = x_le[47 - i];
        out[i + 48] = y_le[47 - i];
    }
    Ok(out)
}

/// Encode a BLS12-381 G2 point to 192 big-endian bytes.
/// EIP-2537 format: (x_c0[48] || x_c1[48] || y_c0[48] || y_c1[48])
fn encode_bls12_g2(point: &Bls12G2) -> Result<[u8; 192], CryptoError> {
    use openvm_algebra_guest::IntMod;
    use openvm_ecc_guest::{weierstrass::WeierstrassPoint, Group};

    if point.is_identity() {
        return Ok([0u8; 192]);
    }

    let x = point.x();
    let y = point.y();
    let x_c0 = x.c0.as_le_bytes();
    let x_c1 = x.c1.as_le_bytes();
    let y_c0 = y.c0.as_le_bytes();
    let y_c1 = y.c1.as_le_bytes();
    let mut out = [0u8; 192];
    for i in 0..48 {
        out[i] = x_c0[47 - i];
        out[i + 48] = x_c1[47 - i];
        out[i + 96] = y_c0[47 - i];
        out[i + 144] = y_c1[47 - i];
    }
    Ok(out)
}

// ── Modexp helpers ────────────────────────────────────────────────────────────

/// Returns true if the modulus (big-endian, possibly with leading zeros) equals BN254 Fr.
fn is_bn254_fr(modulus: &[u8]) -> bool {
    use openvm_algebra_guest::IntMod;

    // Strip leading zeros
    let stripped = match modulus.iter().position(|&b| b != 0) {
        Some(i) => &modulus[i..],
        None => return false, // all zeros
    };
    // bn::Scalar::MODULUS is little-endian; compare against reversed input
    stripped.len() == 32
        && stripped
            .iter()
            .rev()
            .eq(openvm_pairing::bn254::Scalar::MODULUS.as_ref().iter())
}

/// Accelerated modexp for BN254 Fr using field arithmetic intrinsics.
fn accelerated_modexp_bn254_fr(base: &[u8], exp: &[u8]) -> Vec<u8> {
    use openvm_algebra_guest::{IntMod, Reduce};
    use openvm_ecc_guest::algebra::ExpBytes;
    use openvm_pairing::bn254::Scalar as Bn254Scalar;

    let base_fr = if base.len() <= 32 {
        Bn254Scalar::from_be_bytes(base)
            .unwrap_or_else(|| Bn254Scalar::reduce_be_bytes(base))
    } else {
        let padded_len = base.len().next_multiple_of(32);
        let mut padded = vec![0u8; padded_len];
        padded[padded_len - base.len()..].copy_from_slice(base);
        Bn254Scalar::reduce_be_bytes(&padded)
    };

    base_fr.exp_bytes(true, exp).to_be_bytes().as_ref().to_vec()
}

/// Fallback modexp using BigUint (no-std compatible).
fn modexp_fallback(base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
    use num_bigint::BigUint;

    let base_nat = BigUint::from_bytes_be(base);
    let exp_nat = BigUint::from_bytes_be(exp);
    let mod_nat = BigUint::from_bytes_be(modulus);

    let result = if mod_nat == BigUint::ZERO {
        BigUint::ZERO
    } else if exp_nat == BigUint::ZERO {
        BigUint::from(1_u8) % &mod_nat
    } else {
        base_nat.modpow(&exp_nat, &mod_nat)
    };

    let res_bytes = result.to_bytes_be();
    let mut out = vec![0u8; modulus.len()];
    if res_bytes.len() <= modulus.len() {
        let offset = modulus.len() - res_bytes.len();
        out[offset..].copy_from_slice(&res_bytes);
    } else {
        out.copy_from_slice(&res_bytes[res_bytes.len() - modulus.len()..]);
    }
    Ok(out)
}
