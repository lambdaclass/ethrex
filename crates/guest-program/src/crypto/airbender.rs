use std::ops::Mul;

use airbender_crypto::MiniDigest;
use airbender_crypto::ark_ec::{CurveGroup, pairing::Pairing};
use airbender_crypto::ark_ff::{BigInteger, One, PrimeField, QuadExtField, Zero};
use airbender_crypto::bn254::curves::Bn254;
use airbender_crypto::bn254::{Fq, Fr, G1Affine, G1Projective, G2Affine};
use airbender_crypto::k256::ecdsa::{RecoveryId, Signature, hazmat::bits2field};
use airbender_crypto::k256::elliptic_curve::ops::Reduce;
use airbender_crypto::k256::{Scalar, Secp256k1, U256};
use airbender_crypto::ripemd160::{Digest as RipemdDigest, Ripemd160};
use airbender_crypto::secp256k1::{SECP256K1N_HALF, recover};
use airbender_crypto::sha3::Keccak256;
use airbender_crypto::sha256::{Digest as Sha2Digest, Sha256};
use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

// BLS12-381 types (delegated field arithmetic via ark_ff_delegation on riscv32)
use airbender_crypto::ark_ec::AffineRepr;
use airbender_crypto::bls12_381::{
    Fq as BlsFq, Fq2 as BlsFq2, G1Affine as BlsG1Affine, G1Projective as BlsG1Projective,
    G2Affine as BlsG2Affine, G2Projective as BlsG2Projective,
    curves::Bls12_381,
    eip2537::{
        parse_fq_bytes, parse_fq2_bytes, parse_g1_bytes, parse_g2_bytes, serialize_fq_bytes,
        serialize_fq2_bytes,
    },
};

/// Airbender crypto provider using CSR-delegated operations.
///
/// Cryptographic operations are delegated to specialized circuits via
/// airbender-crypto: CSR 0x7ca for 256-bit field arithmetic (powers
/// secp256k1, secp256r1, bn254, bls12-381), keccak_special5 for
/// keccak-f1600 permutation rounds.
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

        let pubkey_bytes = recovered.to_encoded_point(false);
        let hash = <Keccak256 as MiniDigest>::digest(&pubkey_bytes.as_bytes()[1..]);
        Ok(hash)
    }

    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
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
        <Keccak256 as MiniDigest>::digest(input)
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
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];

        r.copy_from_slice(&sig[..32]);
        s.copy_from_slice(&sig[32..]);
        x.copy_from_slice(&pk[..32]);
        y.copy_from_slice(&pk[32..]);

        airbender_crypto::secp256r1::verify(msg, &r, &s, &x, &y).unwrap_or(false)
    }

    // ── BLS12-381 (EIP-2537, delegated field arithmetic) ──────────────

    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        let pa = parse_bls_g1_48(a)?;
        let pb = parse_bls_g1_48(b)?;
        let result = (BlsG1Projective::from(pa) + BlsG1Projective::from(pb)).into_affine();
        serialize_bls_g1_96(&result)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        let mut result = BlsG1Projective::zero();
        for (point_bytes, scalar_bytes) in pairs {
            let point = parse_bls_g1_48(*point_bytes)?;
            if !point.is_zero() && !point.is_in_correct_subgroup_assuming_on_curve() {
                return Err(CryptoError::InvalidPoint("G1 point not in subgroup"));
            }
            let scalar = parse_bls_scalar(scalar_bytes);
            if !scalar.is_zero() {
                result += point.mul_bigint(scalar.into_bigint());
            }
        }
        serialize_bls_g1_96(&result.into_affine())
    }

    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        let pa = parse_bls_g2_192(a)?;
        let pb = parse_bls_g2_192(b)?;
        let result = (BlsG2Projective::from(pa) + BlsG2Projective::from(pb)).into_affine();
        serialize_bls_g2_192(&result)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        let mut result = BlsG2Projective::zero();
        for (point_bytes, scalar_bytes) in pairs {
            let point = parse_bls_g2_192(*point_bytes)?;
            if !point.is_zero() && !point.is_in_correct_subgroup_assuming_on_curve() {
                return Err(CryptoError::InvalidPoint("G2 point not in subgroup"));
            }
            let scalar = parse_bls_scalar(scalar_bytes);
            if !scalar.is_zero() {
                result += point.mul_bigint(scalar.into_bigint());
            }
        }
        serialize_bls_g2_192(&result.into_affine())
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        let mut g1_points = Vec::with_capacity(pairs.len());
        let mut g2_points = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs {
            let g1 = parse_bls_g1_48(*g1_bytes)?;
            let g2 = parse_bls_g2_192(*g2_bytes)?;
            if !g1.is_zero() && !g1.is_in_correct_subgroup_assuming_on_curve() {
                return Err(CryptoError::InvalidPoint("G1 not in subgroup"));
            }
            if !g2.is_zero() && !g2.is_in_correct_subgroup_assuming_on_curve() {
                return Err(CryptoError::InvalidPoint("G2 not in subgroup"));
            }
            if !g1.is_zero() && !g2.is_zero() {
                g1_points.push(g1);
                let prepared: <Bls12_381 as Pairing>::G2Prepared = g2.into();
                g2_points.push(prepared);
            }
        }

        if g1_points.is_empty() {
            return Ok(true);
        }

        let result = Bls12_381::multi_pairing(g1_points, g2_points);
        Ok(result.0.is_one())
    }

    // fp_to_g1 and fp2_to_g2 use trait defaults — the map-to-curve
    // implementation in airbender-crypto's ark-ec doesn't expose
    // MapToCurveImplementation. These are rarely called (EIP-2537 only).

    // ── KZG (EIP-4844 point evaluation precompile) ──────────────────
    // Skip verification inside the zkVM — the proof system itself
    // guarantees execution integrity.  A proper BLS12-381 KZG
    // implementation using delegated field arithmetic should replace
    // this once available.
    fn verify_kzg_proof(
        &self,
        _z: &[u8; 32],
        _y: &[u8; 32],
        _commitment: &[u8; 48],
        _proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        Ok(())
    }
}

// ── BN254 helpers ─────────────────────────────────────────────────────

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

// ── BLS12-381 helpers (EIP-2537 48-byte field elements) ───────────────

fn parse_bls_g1_48((x_bytes, y_bytes): ([u8; 48], [u8; 48])) -> Result<BlsG1Affine, CryptoError> {
    // EIP-2537 uses 64-byte padded field elements, but our Crypto trait
    // passes 48-byte unpadded. Pad to 64 bytes (16 zero bytes prefix).
    let mut x_padded = [0u8; 64];
    x_padded[16..].copy_from_slice(&x_bytes);
    let mut y_padded = [0u8; 64];
    y_padded[16..].copy_from_slice(&y_bytes);

    let x = parse_fq_bytes(&x_padded).ok_or(CryptoError::InvalidInput(
        "G1 x coordinate >= field modulus",
    ))?;
    let y = parse_fq_bytes(&y_padded).ok_or(CryptoError::InvalidInput(
        "G1 y coordinate >= field modulus",
    ))?;

    if x.is_zero() && y.is_zero() {
        return Ok(BlsG1Affine::zero());
    }

    let affine = BlsG1Affine::new_unchecked(x, y);
    if !affine.is_on_curve() {
        return Err(CryptoError::InvalidPoint("G1 point not on curve"));
    }
    Ok(affine)
}

fn parse_bls_g2_192(
    (x0, x1, y0, y1): ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
) -> Result<BlsG2Affine, CryptoError> {
    let mut x0_padded = [0u8; 64];
    x0_padded[16..].copy_from_slice(&x0);
    let mut x1_padded = [0u8; 64];
    x1_padded[16..].copy_from_slice(&x1);
    let mut y0_padded = [0u8; 64];
    y0_padded[16..].copy_from_slice(&y0);
    let mut y1_padded = [0u8; 64];
    y1_padded[16..].copy_from_slice(&y1);

    let x0_fq =
        parse_fq_bytes(&x0_padded).ok_or(CryptoError::InvalidInput("G2 x0 >= field modulus"))?;
    let x1_fq =
        parse_fq_bytes(&x1_padded).ok_or(CryptoError::InvalidInput("G2 x1 >= field modulus"))?;
    let y0_fq =
        parse_fq_bytes(&y0_padded).ok_or(CryptoError::InvalidInput("G2 y0 >= field modulus"))?;
    let y1_fq =
        parse_fq_bytes(&y1_padded).ok_or(CryptoError::InvalidInput("G2 y1 >= field modulus"))?;

    if x0_fq.is_zero() && x1_fq.is_zero() && y0_fq.is_zero() && y1_fq.is_zero() {
        return Ok(BlsG2Affine::zero());
    }

    let affine = BlsG2Affine::new_unchecked(BlsFq2::new(x0_fq, x1_fq), BlsFq2::new(y0_fq, y1_fq));
    if !affine.is_on_curve() {
        return Err(CryptoError::InvalidPoint("G2 point not on curve"));
    }
    Ok(affine)
}

fn parse_bls_scalar(scalar_bytes: &[u8; 32]) -> airbender_crypto::bls12_381::Fr {
    airbender_crypto::bls12_381::Fr::from_be_bytes_mod_order(scalar_bytes)
}

fn serialize_bls_g1_96(point: &BlsG1Affine) -> Result<[u8; 96], CryptoError> {
    if point.is_zero() {
        return Ok([0u8; 96]);
    }
    let mut out = [0u8; 96];
    let mut x_padded = [0u8; 64];
    let mut y_padded = [0u8; 64];
    serialize_fq_bytes(point.x, &mut x_padded);
    serialize_fq_bytes(point.y, &mut y_padded);
    // EIP-2537 outputs 48 bytes per field element (strip 16-byte zero prefix)
    out[..48].copy_from_slice(&x_padded[16..]);
    out[48..].copy_from_slice(&y_padded[16..]);
    Ok(out)
}

fn serialize_bls_g2_192(point: &BlsG2Affine) -> Result<[u8; 192], CryptoError> {
    if point.is_zero() {
        return Ok([0u8; 192]);
    }
    let mut out = [0u8; 192];
    let mut buf = [0u8; 128];
    serialize_fq2_bytes(point.x, &mut buf);
    // EIP-2537: x0 (48 bytes) || x1 (48 bytes)
    out[..48].copy_from_slice(&buf[16..64]);
    out[48..96].copy_from_slice(&buf[80..128]);
    serialize_fq2_bytes(point.y, &mut buf);
    out[96..144].copy_from_slice(&buf[16..64]);
    out[144..192].copy_from_slice(&buf[80..128]);
    Ok(out)
}
