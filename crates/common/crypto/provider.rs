#[cfg(not(feature = "std"))]
use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use ethereum_types::Address;
use sha2::Digest as _;

/// Errors from crypto operations. Opaque — does not leak library-specific types.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid recovery id")]
    InvalidRecoveryId,
    #[error("recovery failed")]
    RecoveryFailed,
    #[error("invalid point: {0}")]
    InvalidPoint(&'static str),
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("verification failed")]
    VerificationFailed,
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    #[error("{0}")]
    Other(String),
}

/// Error returned by the BLS12-381 trait defaults when no backend is available:
/// the host backend (`blst`) is compiled out and no provider override is in
/// place. zkVM guest providers override these methods, so this is never hit on
/// the guest; on the host the `blst` feature (default-on) supplies the backend.
#[cfg(not(feature = "blst"))]
const BLS_UNSUPPORTED: &str =
    "bls12_381 requires the `blst` feature (host/L1) or a zkVM provider override";

/// All cryptographic operations the EVM needs.
///
/// Implementors provide the actual crypto — native libraries, zkVM circuits,
/// or anything else. ethrex's EVM code depends only on this trait.
///
/// Default implementations use native system libraries. Implementors only
/// override methods where they need different behavior (e.g. zkVM-accelerated
/// ECDSA or pairing checks).
///
/// Methods take `&self` to support `&dyn Crypto` (dynamic dispatch).
/// Implementations are typically zero-sized structs.
///
/// # zkVM implementors
///
/// The following methods **must** be overridden for zkVM targets, as their
/// default implementations use native C libraries (secp256k1, ark-bn254, etc.)
/// that cannot run inside a zkVM guest:
///
/// - [`secp256k1_ecrecover`](Crypto::secp256k1_ecrecover) — uses `libsecp256k1` C library
/// - [`recover_signer`](Crypto::recover_signer) — uses `libsecp256k1` C library
/// - [`verify_signature`](Crypto::verify_signature) — uses `libsecp256k1` C library
/// - [`bn254_g1_add`](Crypto::bn254_g1_add), [`bn254_g1_mul`](Crypto::bn254_g1_mul),
///   [`bn254_pairing_check`](Crypto::bn254_pairing_check) — use `ark-bn254`
/// - [`bls12_381_g1_add`](Crypto::bls12_381_g1_add), [`bls12_381_g2_add`](Crypto::bls12_381_g2_add),
///   [`bls12_381_g1_msm`](Crypto::bls12_381_g1_msm), [`bls12_381_g2_msm`](Crypto::bls12_381_g2_msm),
///   [`bls12_381_pairing_check`](Crypto::bls12_381_pairing_check) — use `bls12_381` crate
///   [`bls12_381_map_fp_to_g1`](Crypto::bls12_381_map_fp_to_g1),
///   [`bls12_381_map_fp2_to_g2`](Crypto::bls12_381_map_fp2_to_g2) — use `bls12_381` crate
///
/// Non-overridden methods will silently use the native default, which will
/// fail to compile or panic at runtime inside a zkVM guest.
pub trait Crypto: Send + Sync + core::fmt::Debug {
    // ── ECDSA (secp256k1) ──────────────────────────────────────────────

    /// Recover the Ethereum address from a 64-byte signature + recovery id + 32-byte message hash.
    /// Used by the ECRECOVER precompile (0x01).
    /// Returns the 32-byte keccak hash of the uncompressed public key (address is last 20 bytes).
    #[cfg(feature = "secp256k1")]
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

        let hash = crate::keccak::keccak_hash(&public_key.serialize_uncompressed()[1..]);
        Ok(hash)
    }

    #[cfg(not(feature = "secp256k1"))]
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        use k256::{
            AffinePoint, ProjectivePoint, Scalar,
            elliptic_curve::{
                PrimeField,
                group::prime::PrimeCurveAffine,
                ops::{Invert, LinearCombination, Reduce},
                point::DecompressPoint,
                sec1::ToEncodedPoint,
            },
        };

        // Parse r and s as scalars, rejecting values >= curve order
        let r_bytes = k256::FieldBytes::from_slice(&sig[..32]);
        let s_bytes = k256::FieldBytes::from_slice(&sig[32..]);
        let r: Option<Scalar> = Scalar::from_repr(*r_bytes).into();
        let s: Option<Scalar> = Scalar::from_repr(*s_bytes).into();

        let (Some(r), Some(s)) = (r, s) else {
            return Err(CryptoError::InvalidSignature);
        };

        if r.is_zero().into() || s.is_zero().into() {
            return Err(CryptoError::InvalidSignature);
        }

        // Decompress R from r and recovery id parity.
        // Note: recid >= 2 means R.x = r + n (curve order), which has ~2^-128
        // probability on secp256k1 and never occurs in practice. We don't handle
        // it here — decompression will simply fail and return RecoveryFailed.
        let y_is_odd = (recid & 1) != 0;
        let r_point: Option<AffinePoint> =
            AffinePoint::decompress(r_bytes, u8::from(y_is_odd).into()).into();
        let Some(r_point) = r_point else {
            return Err(CryptoError::RecoveryFailed);
        };

        // Recover public key: pk = r^(-1) * (s*R - z*G)
        let r_proj = ProjectivePoint::from(r_point);
        let z = <Scalar as Reduce<k256::U256>>::reduce_bytes(k256::FieldBytes::from_slice(msg));
        let r_inv: Option<Scalar> = r.invert_vartime().into();
        let Some(r_inv) = r_inv else {
            return Err(CryptoError::RecoveryFailed);
        };
        let u1 = -(r_inv * z);
        let u2 = r_inv * s;
        let pk = ProjectivePoint::lincomb(&ProjectivePoint::GENERATOR, &u1, &r_proj, &u2);

        let pk_affine = pk.to_affine();
        if bool::from(pk_affine.is_identity()) {
            return Err(CryptoError::RecoveryFailed);
        }
        let uncompressed = pk_affine.to_encoded_point(false);
        let hash = crate::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
        Ok(hash)
    }

    /// Recover the signer address from a 65-byte signature (r||s||v) + 32-byte message hash.
    /// Used by transaction validation (tx.sender()) and EIP-7702 authority recovery.
    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
        // EIP-2: reject high-s signatures (s > secp256k1n/2)
        const SECP256K1_N_HALF: [u8; 32] =
            hex_literal::hex!("7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0");
        if sig[32..64] > SECP256K1_N_HALF[..] {
            return Err(CryptoError::InvalidSignature);
        }

        let hash = self.secp256k1_ecrecover(
            sig[..64]
                .try_into()
                .map_err(|_| CryptoError::InvalidSignature)?,
            sig[64],
            msg,
        )?;
        Ok(Address::from_slice(&hash[12..]))
    }

    /// Verify `sig` (r||s||v) against `public_key`, bound to recovery id `v`.
    /// Used by EIP-8025 sender hints.
    #[cfg(feature = "secp256k1")]
    fn verify_signature(&self, sig: &[u8; 65], msg: &[u8; 32], public_key: &[u8; 65]) -> bool {
        if !signature_preflight_ok(sig) {
            return false;
        }
        let Ok(recovery_id) = secp256k1::ecdsa::RecoveryId::try_from(sig[64] as i32) else {
            return false;
        };
        let Ok(recoverable_sig) =
            secp256k1::ecdsa::RecoverableSignature::from_compact(&sig[0..64], recovery_id)
        else {
            return false;
        };
        let message = secp256k1::Message::from_digest(*msg);
        let Ok(expected_pk) = secp256k1::PublicKey::from_slice(public_key) else {
            return false;
        };
        // Recover with `v` and compare; plain verify accepts either candidate.
        match recoverable_sig.recover(&message) {
            Ok(recovered_pk) => recovered_pk == expected_pk,
            Err(_) => false,
        }
    }

    /// Verify `sig` (r||s||v) against `public_key`, bound to recovery id `v`.
    /// Used by EIP-8025 sender hints.
    #[cfg(not(feature = "secp256k1"))]
    fn verify_signature(&self, sig: &[u8; 65], msg: &[u8; 32], public_key: &[u8; 65]) -> bool {
        use k256::{
            ProjectivePoint, Scalar,
            ecdsa::VerifyingKey,
            elliptic_curve::{
                PrimeField,
                group::prime::PrimeCurveAffine,
                ops::{Invert, LinearCombination, Reduce},
                point::AffineCoordinates,
            },
        };

        if !signature_preflight_ok(sig) {
            return false;
        }

        let r_bytes = k256::FieldBytes::from_slice(&sig[..32]);
        let s_bytes = k256::FieldBytes::from_slice(&sig[32..64]);
        let r: Option<Scalar> = Scalar::from_repr(*r_bytes).into();
        let s: Option<Scalar> = Scalar::from_repr(*s_bytes).into();
        let (Some(r), Some(s)) = (r, s) else {
            return false;
        };
        if r.is_zero().into() || s.is_zero().into() {
            return false;
        }

        let Ok(vk) = VerifyingKey::from_sec1_bytes(public_key) else {
            return false;
        };
        let q = ProjectivePoint::from(*vk.as_affine());

        // R' = s⁻¹·(z·G + r·Q); accept iff R'.x == r and R'.y parity matches `v`.
        let z = <Scalar as Reduce<k256::U256>>::reduce_bytes(k256::FieldBytes::from_slice(msg));
        let s_inv: Option<Scalar> = s.invert_vartime().into();
        let Some(s_inv) = s_inv else {
            return false;
        };
        let u1 = z * s_inv;
        let u2 = r * s_inv;
        let big_r = ProjectivePoint::lincomb(&ProjectivePoint::GENERATOR, &u1, &q, &u2).to_affine();
        if bool::from(big_r.is_identity()) {
            return false;
        }
        // Compare R'.x to r as field bytes, without reducing mod n. A nonce point
        // with x = r + n (possible only when r < p - n) must be rejected to match
        // ecrecover, which reconstructs R from x = r exactly; reducing mod n here
        // would accept that aliasing and diverge from canonical recovery.
        if big_r.x().as_slice() != &sig[..32] {
            return false;
        }
        bool::from(big_r.y_is_odd()) == (sig[64] == 1)
    }

    // ── Hashing ────────────────────────────────────────────────────────

    /// Keccak-256 hash. Used by the KECCAK256 opcode (0x20) and address derivation.
    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        crate::keccak::keccak_hash(input)
    }

    /// SHA-256 hash. Used by SHA2-256 precompile (0x02) and KZG point evaluation.
    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        sha2::Sha256::digest(input).into()
    }

    /// RIPEMD-160 hash (zero-padded to 32 bytes). Used by RIPEMD-160 precompile (0x03).
    fn ripemd160(&self, input: &[u8]) -> [u8; 32] {
        let mut hasher = ripemd::Ripemd160::new();
        hasher.update(input);
        let result = hasher.finalize();

        let mut output = [0u8; 32];
        output[12..].copy_from_slice(&result);
        output
    }

    // ── BN254 (alt_bn128) ──────────────────────────────────────────────

    /// G1 point addition. Used by ECADD precompile (0x06).
    /// Input: two uncompressed G1 points (64 bytes each as big-endian x||y).
    /// Output: uncompressed G1 point (64 bytes).
    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        use ark_bn254::Fq;
        use ark_ec::CurveGroup;
        use ark_ff::{BigInteger, PrimeField as _, Zero};

        let parse_point = |bytes: &[u8]| -> Result<ark_bn254::G1Affine, CryptoError> {
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

    /// G1 scalar multiplication. Used by ECMUL precompile (0x07).
    /// Input: uncompressed G1 point (64 bytes) + scalar (32 bytes big-endian).
    /// Output: uncompressed G1 point (64 bytes).
    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        use ark_bn254::{Fq, Fr as FrArk};
        use ark_ec::CurveGroup;
        use ark_ff::{BigInteger, PrimeField as _, Zero};
        use core::ops::Mul as _;

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

    /// Pairing check. Used by ECPAIRING precompile (0x08).
    /// Input: pairs of (G1 64 bytes, G2 128 bytes) as raw byte slices.
    /// Returns true if the pairing equation holds.
    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        use ark_bn254::{Bn254, Fq, G1Affine, G2Affine};
        use ark_ec::pairing::Pairing;
        use ark_ff::{One, PrimeField as _, QuadExtField, Zero};

        let mut g1_points = Vec::with_capacity(pairs.len());
        let mut g2_points = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs {
            if g1_bytes.len() < 64 {
                return Err(CryptoError::InvalidInput("G1 must be 64 bytes"));
            }
            let g1x = Fq::from_be_bytes_mod_order(&g1_bytes[..32]);
            let g1y = Fq::from_be_bytes_mod_order(&g1_bytes[32..64]);

            let g1 = if g1x.is_zero() && g1y.is_zero() {
                G1Affine::identity()
            } else {
                let p = G1Affine::new_unchecked(g1x, g1y);
                if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
                    return Err(CryptoError::InvalidPoint("G1 not on BN254 curve"));
                }
                p
            };
            g1_points.push(g1);

            if g2_bytes.len() < 128 {
                return Err(CryptoError::InvalidInput("G2 must be 128 bytes"));
            }

            let g2_x_im = Fq::from_be_bytes_mod_order(&g2_bytes[..32]);
            let g2_x_re = Fq::from_be_bytes_mod_order(&g2_bytes[32..64]);
            let g2_y_im = Fq::from_be_bytes_mod_order(&g2_bytes[64..96]);
            let g2_y_re = Fq::from_be_bytes_mod_order(&g2_bytes[96..128]);

            let g2 =
                if g2_x_im.is_zero() && g2_x_re.is_zero() && g2_y_im.is_zero() && g2_y_re.is_zero()
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

    /// Modular exponentiation (arbitrary precision).
    /// Used by MODEXP precompile (0x05).
    #[cfg(feature = "std")]
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

        let res_bytes: Vec<u8> = result.to_power_of_2_digits_desc(8);
        pad_modexp_output(res_bytes, modulus.len())
    }

    #[cfg(not(feature = "std"))]
    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
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
        pad_modexp_output(res_bytes, modulus.len())
    }

    /// 256-bit modular multiplication.
    /// Used by the MULMOD opcode. Default impl uses standard bigint arithmetic.
    /// ZisK overrides with a native circuit instruction.
    fn mulmod256(&self, a: &[u8; 32], b: &[u8; 32], m: &[u8; 32]) -> [u8; 32] {
        let a = ethereum_types::U256::from_big_endian(a);
        let b = ethereum_types::U256::from_big_endian(b);
        let m = ethereum_types::U256::from_big_endian(m);

        let result = if m.is_zero() {
            ethereum_types::U256::zero()
        } else {
            let product = a.full_mul(b);
            let m512 = ethereum_types::U512::from(m);
            if product < m512 {
                // Product fits below modulus — no division needed.
                product.try_into().unwrap_or(ethereum_types::U256::zero())
            } else {
                let (_, rem) = product.div_mod(m512);
                rem.try_into().unwrap_or(ethereum_types::U256::zero())
            }
        };

        result.to_big_endian()
    }

    // ── Blake2 ─────────────────────────────────────────────────────────

    /// Blake2b compression function F. Used by BLAKE2F precompile (0x09).
    fn blake2_compress(&self, rounds: u32, h: &mut [u64; 8], m: [u64; 16], t: [u64; 2], f: bool) {
        #[allow(clippy::as_conversions)]
        crate::blake2f::blake2b_f(rounds as usize, h, &m, &t, f);
    }

    // ── secp256r1 (P-256) ──────────────────────────────────────────────

    /// P-256 signature verification. Used by P256VERIFY precompile (0x0100, Osaka).
    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        use p256::{
            EncodedPoint,
            ecdsa::{Signature as P256Signature, signature::hazmat::PrehashVerifier},
            elliptic_curve::bigint::U256 as P256Uint,
        };

        let r = P256Uint::from_be_slice(&sig[..32]);
        let s = P256Uint::from_be_slice(&sig[32..]);

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

    /// KZG point evaluation. Used by POINT_EVALUATION precompile (0x0a, Cancun).
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
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        crate::kzg::verify_kzg_proof(*commitment, *z, *y, *proof)
            .map_err(|e| CryptoError::Other(e.to_string()))
            .and_then(|valid| {
                if valid {
                    Ok(())
                } else {
                    Err(CryptoError::VerificationFailed)
                }
            })
    }

    /// Verify blob KZG proof. Used by blob transaction validation.
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
            .verify_blob_kzg_proof(&blob_arr.into(), &(*commitment).into(), &(*proof).into())
            .map_err(|e| CryptoError::Other(e.to_string()))
    }

    #[cfg(not(feature = "c-kzg"))]
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

        crate::kzg::verify_blob_kzg_proof(blob_arr, *commitment, *proof)
            .map_err(|e| CryptoError::Other(e.to_string()))
    }

    // ── BLS12-381 (Prague, EIP-2537) ───────────────────────────────────

    // The BLS12-381 (EIP-2537) operations default to the assembly-optimized
    // `blst` backend on the host (the `blst` feature, default-on). When `blst`
    // is compiled out — i.e. zkVM guest builds — these defaults return an error
    // and the guest `Crypto` providers (Sp1/Risc0/OpenVm via the portable
    // `bls12_381` crate, Zisk via FFI) override every one of them. The portable
    // pure-Rust implementation lives in `ethrex-guest-program`, so the published
    // `ethrex-crypto` crate carries no git dependency.

    /// G1 addition. Returns 96-byte unpadded G1 point.
    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        #[cfg(feature = "blst")]
        {
            crate::bls_blst::g1_add(a, b)
        }
        #[cfg(not(feature = "blst"))]
        {
            let _ = (a, b);
            Err(CryptoError::Unsupported(BLS_UNSUPPORTED))
        }
    }

    /// G1 multi-scalar multiplication. Returns 96-byte unpadded G1 point.
    #[allow(clippy::type_complexity)]
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        #[cfg(feature = "blst")]
        {
            crate::bls_blst::g1_msm(pairs)
        }
        #[cfg(not(feature = "blst"))]
        {
            let _ = pairs;
            Err(CryptoError::Unsupported(BLS_UNSUPPORTED))
        }
    }

    /// G2 addition. Returns 192-byte unpadded G2 point.
    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        #[cfg(feature = "blst")]
        {
            crate::bls_blst::g2_add(a, b)
        }
        #[cfg(not(feature = "blst"))]
        {
            let _ = (a, b);
            Err(CryptoError::Unsupported(BLS_UNSUPPORTED))
        }
    }

    /// G2 multi-scalar multiplication. Returns 192-byte unpadded G2 point.
    #[allow(clippy::type_complexity)]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        #[cfg(feature = "blst")]
        {
            crate::bls_blst::g2_msm(pairs)
        }
        #[cfg(not(feature = "blst"))]
        {
            let _ = pairs;
            Err(CryptoError::Unsupported(BLS_UNSUPPORTED))
        }
    }

    /// BLS12-381 pairing check.
    #[allow(clippy::type_complexity)]
    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        #[cfg(feature = "blst")]
        {
            crate::bls_blst::pairing_check(pairs)
        }
        #[cfg(not(feature = "blst"))]
        {
            let _ = pairs;
            Err(CryptoError::Unsupported(BLS_UNSUPPORTED))
        }
    }

    /// Map field element to G1 point.
    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        #[cfg(feature = "blst")]
        {
            crate::bls_blst::fp_to_g1(fp)
        }
        #[cfg(not(feature = "blst"))]
        {
            let _ = fp;
            Err(CryptoError::Unsupported(BLS_UNSUPPORTED))
        }
    }

    /// Map field element pair to G2 point.
    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        #[cfg(feature = "blst")]
        {
            crate::bls_blst::fp2_to_g2(fp2)
        }
        #[cfg(not(feature = "blst"))]
        {
            let _ = fp2;
            Err(CryptoError::Unsupported(BLS_UNSUPPORTED))
        }
    }
}

// ── verify_signature helpers ───────────────────────────────────────────────

/// EIP-2 low-s and canonical recovery-byte preflight for `verify_signature`.
/// Returns true iff `s <= n/2` and `v ∈ {0, 1}`.
fn signature_preflight_ok(sig: &[u8; 65]) -> bool {
    const SECP256K1_N_HALF: [u8; 32] =
        hex_literal::hex!("7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0");
    sig[32..64] <= SECP256K1_N_HALF[..] && sig[64] <= 1
}

// ── Modexp helper ──────────────────────────────────────────────────────────

/// Pad or truncate modexp result bytes to match the modulus length.
fn pad_modexp_output(res_bytes: Vec<u8>, modulus_len: usize) -> Result<Vec<u8>, CryptoError> {
    let mut out = vec![0u8; modulus_len];
    if res_bytes.len() <= modulus_len {
        let offset = modulus_len - res_bytes.len();
        out[offset..].copy_from_slice(&res_bytes);
    } else {
        out.copy_from_slice(&res_bytes[res_bytes.len() - modulus_len..]);
    }
    Ok(out)
}

// The BLS12-381 point parse/serialize helpers that backed the portable trait
// defaults now live in `ethrex-guest-program` alongside the guest `Crypto`
// providers, so this crate no longer depends on the `bls12_381` git fork.

#[cfg(test)]
mod tests {
    //! Tests for `Crypto::verify_signature`. Exercise whichever backend
    //! (native `secp256k1` or `k256`) is currently compiled. Signing always
    //! uses `k256` so the same source covers both backends.

    use super::Crypto;
    use crate::NativeCrypto;
    use ethereum_types::U256 as EthU256;
    use hex_literal::hex;
    use k256::{
        AffinePoint, FieldBytes, ProjectivePoint, Scalar, U256,
        ecdsa::SigningKey,
        elliptic_curve::{PrimeField, ops::Reduce, point::DecompressPoint, sec1::ToEncodedPoint},
    };

    const SECP256K1_N: [u8; 32] =
        hex!("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141");

    fn sign(sk: &[u8; 32], msg: &[u8; 32]) -> ([u8; 65], [u8; 65]) {
        let signing_key = SigningKey::from_bytes(sk.into()).unwrap();
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(msg).unwrap();

        let mut sig = [0u8; 65];
        sig[..64].copy_from_slice(&signature.to_bytes());
        sig[64] = recovery_id.to_byte();

        let pk_uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let pk: [u8; 65] = pk_uncompressed.as_bytes().try_into().unwrap();

        (sig, pk)
    }

    fn test_key() -> [u8; 32] {
        [7u8; 32]
    }

    #[test]
    fn verify_signature_accepts_valid() {
        let msg = [0x11u8; 32];
        let (sig, pk) = sign(&test_key(), &msg);
        assert!(NativeCrypto.verify_signature(&sig, &msg, &pk));
    }

    /// Flipping the recovery id must make verification fail. This is the
    /// property that forces both backends to bind to `v` rather than
    /// accept either parity candidate.
    #[test]
    fn verify_signature_binds_recovery_id() {
        let msg = [0x11u8; 32];
        let (mut sig, pk) = sign(&test_key(), &msg);
        assert!(NativeCrypto.verify_signature(&sig, &msg, &pk));
        sig[64] ^= 1;
        assert!(!NativeCrypto.verify_signature(&sig, &msg, &pk));
    }

    #[test]
    fn verify_signature_rejects_wrong_public_key() {
        let msg = [0x11u8; 32];
        let (sig, _) = sign(&test_key(), &msg);
        let (_, other_pk) = sign(&[9u8; 32], &msg);
        assert!(!NativeCrypto.verify_signature(&sig, &msg, &other_pk));
    }

    /// EIP-2: signatures with `s > n/2` must be rejected.
    #[test]
    fn verify_signature_rejects_high_s() {
        let msg = [0x11u8; 32];
        let (mut sig, pk) = sign(&test_key(), &msg);
        let n = EthU256::from_big_endian(&SECP256K1_N);
        let s = EthU256::from_big_endian(&sig[32..64]);
        sig[32..64].copy_from_slice(&(n - s).to_big_endian());
        assert!(!NativeCrypto.verify_signature(&sig, &msg, &pk));
    }

    /// The recovery byte must be 0 or 1.
    #[test]
    fn verify_signature_rejects_out_of_range_recovery_byte() {
        let msg = [0x11u8; 32];
        let (mut sig, pk) = sign(&test_key(), &msg);
        sig[64] = 2;
        assert!(!NativeCrypto.verify_signature(&sig, &msg, &pk));
    }

    /// Hand-construct a forgery where the nonce point's x coordinate equals
    /// `r + n` (in Fp) — possible only when `r < p - n ≈ 2^128`. Both
    /// backends must reject:
    ///
    /// - The k256 backend's byte comparison `R'.x.as_slice() != sig[..32]`
    ///   catches the aliasing; a "simplification" to compare scalars mod n
    ///   would silently accept this vector.
    /// - The native backend recovers R from `x = r` exactly, yielding a
    ///   different recovered key than the forged Q.
    #[test]
    fn verify_signature_rejects_x_plus_n_aliasing() {
        // Add i (1..=200) to SECP256K1_N big-endian, no overflow since
        // n + 200 < p. Whether x = r + n is on the curve is ~50% per r,
        // so a small search suffices.
        fn add_small(base: &[u8; 32], i: u16) -> [u8; 32] {
            let mut out = *base;
            let mut carry = i as u32;
            for byte in out.iter_mut().rev() {
                let sum = *byte as u32 + carry;
                *byte = (sum & 0xff) as u8;
                carry = sum >> 8;
                if carry == 0 {
                    break;
                }
            }
            out
        }

        let (r_bytes, big_r) = (1..=200u16)
            .find_map(|i| {
                let mut r_bytes = [0u8; 32];
                r_bytes[30..].copy_from_slice(&i.to_be_bytes());
                let x_bytes = add_small(&SECP256K1_N, i);
                let candidate: Option<AffinePoint> =
                    AffinePoint::decompress(FieldBytes::from_slice(&x_bytes), 0u8.into()).into();
                candidate.map(|p| (r_bytes, p))
            })
            .expect("expected a small r with x = r + n on the curve");

        let r: Scalar = Scalar::from_repr(*FieldBytes::from_slice(&r_bytes))
            .into_option()
            .unwrap();

        // Arbitrary low-s s and message hash z.
        let mut s_bytes = [0u8; 32];
        s_bytes[31] = 0x42;
        let s: Scalar = Scalar::from_repr(*FieldBytes::from_slice(&s_bytes))
            .into_option()
            .unwrap();
        let z_bytes = [0x99u8; 32];
        let z = <Scalar as Reduce<U256>>::reduce_bytes(FieldBytes::from_slice(&z_bytes));

        // Q = r⁻¹ · (s·R - z·G) makes R' = s⁻¹(z·G + r·Q) equal R exactly.
        // A buggy verify that reduced R'.x mod n would see `(r+n) mod n = r`
        // and accept; the byte-compare guard must reject.
        let r_inv: Scalar = r.invert().into_option().unwrap();
        let big_r_proj = ProjectivePoint::from(big_r);
        let g = ProjectivePoint::GENERATOR;
        let q = (big_r_proj * s - g * z) * r_inv;
        let q_encoded = q.to_affine().to_encoded_point(false);
        let pk_bytes: [u8; 65] = q_encoded.as_bytes().try_into().unwrap();

        // Signature uses r (NOT r + n) and v = 0.
        let mut sig = [0u8; 65];
        sig[..32].copy_from_slice(&r_bytes);
        sig[32..64].copy_from_slice(&s_bytes);
        sig[64] = 0;

        assert!(
            !NativeCrypto.verify_signature(&sig, &z_bytes, &pk_bytes),
            "verify_signature must reject the x = r + n forgery"
        );
    }

    /// The k256 backend rejects hybrid (`0x06`/`0x07`) SEC1 prefixes via
    /// `VerifyingKey::from_sec1_bytes`. Pins the divergence the
    /// `Transaction::compute_sender_with_hint` `0x04` guard exists for; the
    /// native backend instead accepts hybrid (see the sibling test under
    /// `test/tests/crypto/`).
    #[test]
    #[cfg(not(feature = "secp256k1"))]
    fn verify_signature_rejects_hybrid_encoded_key() {
        let msg = [0x11u8; 32];
        let (sig, mut pk) = sign(&test_key(), &msg);
        assert!(NativeCrypto.verify_signature(&sig, &msg, &pk));

        // Re-tag as hybrid matching Y's parity.
        pk[0] = if pk[64] & 1 == 1 { 0x07 } else { 0x06 };
        assert!(!NativeCrypto.verify_signature(&sig, &msg, &pk));
    }
}
