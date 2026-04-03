use std::ops::Mul;

use airbender_crypto::MiniDigest;
use airbender_crypto::ark_ec::{CurveGroup, pairing::Pairing};
use airbender_crypto::ark_ff::{BigInteger, One, PrimeField, QuadExtField, Zero};
use airbender_crypto::bn254::curves::Bn254;
use airbender_crypto::bn254::{Fq, Fr, G1Affine, G1Projective, G2Affine};
use airbender_crypto::k256::ecdsa::{RecoveryId, Signature, hazmat::bits2field};
use airbender_crypto::k256::elliptic_curve::ops::Reduce;
use airbender_crypto::k256::{Scalar, Secp256k1, U256};
use airbender_crypto::p256::ecdsa::{
    Signature as P256Signature, signature::hazmat::PrehashVerifier,
};
use airbender_crypto::p256::{EncodedPoint, elliptic_curve::bigint::U256 as P256Uint};
use airbender_crypto::ripemd160::{Digest as RipemdDigest, Ripemd160};
use airbender_crypto::secp256k1::{SECP256K1N_HALF, recover};
use airbender_crypto::sha3::Keccak256;
use airbender_crypto::sha256::{Digest as Sha2Digest, Sha256};
use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

/// Airbender crypto provider using CSR-delegated operations.
///
/// Cryptographic operations are delegated to specialized circuits via
/// airbender-crypto: CSR 0x7ca for 256-bit field arithmetic (powers
/// secp256k1, bn254, bls12-381), keccak_special5 for keccak-f1600
/// permutation rounds.
///
/// This module is compiled into the guest binary where `airbender-crypto`
/// is available. It is NOT used on the host side — the host uses
/// `airbender-build-elf` which only triggers ELF compilation.
#[derive(Debug)]
pub struct AirbenderCrypto;

impl Crypto for AirbenderCrypto {
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        let mut sig_obj = Signature::from_slice(sig).map_err(|_| CryptoError::InvalidSignature)?;

        let mut recid_byte = recid;
        if let Some(low_s) = sig_obj.normalize_s() {
            sig_obj = low_s;
            recid_byte ^= 1;
        }

        let recovery_id =
            RecoveryId::from_byte(recid_byte).ok_or(CryptoError::InvalidRecoveryId)?;
        let msg_scalar = <Scalar as Reduce<U256>>::reduce_bytes(
            &bits2field::<Secp256k1>(msg).map_err(|_| CryptoError::RecoveryFailed)?,
        );

        let recovered = recover(&msg_scalar, &sig_obj, &recovery_id)
            .map_err(|_| CryptoError::RecoveryFailed)?;

        // Uncompressed SEC1 encoding: 04 || x (32 bytes) || y (32 bytes)
        let pubkey_bytes = recovered.to_encoded_point(false);
        // Ethereum address = keccak256(x || y)[12..], skip the 0x04 prefix
        let hash = ethrex_crypto::keccak::keccak_hash(&pubkey_bytes.as_bytes()[1..]);
        Ok(hash)
    }

    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
        // EIP-2: reject high-s signatures
        if sig[32..64] > SECP256K1N_HALF[..] {
            return Err(CryptoError::InvalidSignature);
        }

        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&sig[..64]);
        let recid = sig[64];

        let hash = self.secp256k1_ecrecover(&sig_bytes, recid, msg)?;
        Ok(Address::from_slice(&hash[12..]))
    }

    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        // Use the same keccak as the trie (tiny_keccak) to rule out
        // delegation issues. The delegated Keccak256 from airbender-crypto
        // uses CSR keccak_special5 which may produce wrong results.
        ethrex_crypto::keccak::keccak_hash(input)
    }

    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        Sha256::digest(input).into()
    }

    fn ripemd160(&self, input: &[u8]) -> [u8; 32] {
        let result = Ripemd160::digest(input);
        let mut output = [0u8; 32];
        output[12..].copy_from_slice(&result);
        output
    }

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        let pt1 = parse_bn254_g1(p1)?;
        let pt2 = parse_bn254_g1(p2)?;
        let sum = (G1Projective::from(pt1) + G1Projective::from(pt2)).into_affine();
        Ok(serialize_bn254_g1(&sum))
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        if point.len() < 64 || scalar.len() < 32 {
            return Err(CryptoError::InvalidInput("invalid input length"));
        }

        let x = Fq::from_be_bytes_mod_order(&point[..32]);
        let y = Fq::from_be_bytes_mod_order(&point[32..64]);
        if x.is_zero() && y.is_zero() {
            return Ok([0u8; 64]);
        }

        let pt = G1Affine::new_unchecked(x, y);
        if !pt.is_on_curve() {
            return Err(CryptoError::InvalidPoint("G1 point not on curve"));
        }

        let s = Fr::from_be_bytes_mod_order(scalar);
        if s.is_zero() {
            return Ok([0u8; 64]);
        }

        let result = pt.mul(s).into_affine();
        Ok(serialize_bn254_g1(&result))
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        let mut g1_points = Vec::with_capacity(pairs.len());
        let mut g2_points = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs {
            g1_points.push(parse_bn254_g1(g1_bytes)?);

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

    fn blake2_compress(&self, rounds: u32, h: &mut [u64; 8], m: [u64; 16], t: [u64; 2], f: bool) {
        #[allow(clippy::as_conversions)]
        ethrex_crypto::blake2f::blake2b_f(rounds as usize, h, &m, &t, f)
    }

    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
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

        let Ok(verifier) = airbender_crypto::p256::ecdsa::VerifyingKey::from_encoded_point(
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
}

fn parse_bn254_g1(bytes: &[u8]) -> Result<G1Affine, CryptoError> {
    if bytes.len() < 64 {
        return Err(CryptoError::InvalidInput("G1 point must be 64 bytes"));
    }
    let x = Fq::from_be_bytes_mod_order(&bytes[..32]);
    let y = Fq::from_be_bytes_mod_order(&bytes[32..64]);
    if x.is_zero() && y.is_zero() {
        return Ok(G1Affine::identity());
    }
    let point = G1Affine::new_unchecked(x, y);
    if !point.is_on_curve() {
        return Err(CryptoError::InvalidPoint("G1 point not on curve"));
    }
    Ok(point)
}

fn serialize_bn254_g1(point: &G1Affine) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&point.x.into_bigint().to_bytes_be());
    out[32..].copy_from_slice(&point.y.into_bigint().to_bytes_be());
    out
}
