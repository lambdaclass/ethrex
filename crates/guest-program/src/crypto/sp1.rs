use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError, NativeCrypto};

/// SP1 crypto provider.
///
/// Uses k256 for ECDSA (secp256k1) and substrate-bn for BN254 ecmul/pairing.
/// Delegates all other operations to [`NativeCrypto`].
///
/// When building actual SP1 guest binaries, SP1's patched crate versions
/// of k256 and substrate-bn are used transparently via Cargo patches.
#[derive(Debug)]
pub struct Sp1Crypto;

impl Crypto for Sp1Crypto {
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

    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        NativeCrypto.sha256(input)
    }

    fn ripemd160(&self, input: &[u8]) -> [u8; 32] {
        NativeCrypto.ripemd160(input)
    }

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        NativeCrypto.bn254_g1_add(p1, p2)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        substrate_bn_g1_mul(point, scalar)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        substrate_bn_pairing_check(pairs)
    }

    fn modexp(
        &self,
        base: &[u8],
        exp: &[u8],
        modulus: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        NativeCrypto.modexp(base, exp, modulus)
    }

    fn blake2_compress(
        &self,
        rounds: u32,
        h: &mut [u64; 8],
        m: [u64; 16],
        t: [u64; 2],
        f: bool,
    ) {
        NativeCrypto.blake2_compress(rounds, h, m, t, f)
    }

    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        NativeCrypto.secp256r1_verify(msg, sig, pk)
    }

    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        NativeCrypto.verify_kzg_proof(z, y, commitment, proof)
    }

    fn verify_blob_kzg_proof(
        &self,
        blob: &[u8],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<bool, CryptoError> {
        NativeCrypto.verify_blob_kzg_proof(blob, commitment, proof)
    }

    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        NativeCrypto.bls12_381_g1_add(a, b)
    }

    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        NativeCrypto.bls12_381_g1_msm(pairs)
    }

    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        NativeCrypto.bls12_381_g2_add(a, b)
    }

    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        NativeCrypto.bls12_381_g2_msm(pairs)
    }

    fn bls12_381_pairing_check(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), ([u8; 48], [u8; 48], [u8; 48], [u8; 48]))],
    ) -> Result<bool, CryptoError> {
        NativeCrypto.bls12_381_pairing_check(pairs)
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        NativeCrypto.bls12_381_fp_to_g1(fp)
    }

    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        NativeCrypto.bls12_381_fp2_to_g2(fp2)
    }
}

// ── Shared k256 implementations ──────────────────────────────────────────

/// ECDSA public key recovery using k256 (pure Rust, RISC-V compatible).
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

    let recovery_id =
        RecoveryId::from_byte(recid_byte).ok_or(CryptoError::InvalidRecoveryId)?;

    let vk = VerifyingKey::recover_from_prehash(msg, &sig_obj, recovery_id)
        .map_err(|_| CryptoError::RecoveryFailed)?;

    // SEC1 uncompressed: 0x04 || X(32) || Y(32). We need keccak(X||Y).
    let uncompressed = vk.to_encoded_point(false);
    let uncompressed_bytes = uncompressed.as_bytes();
    #[allow(clippy::indexing_slicing)]
    let xy = &uncompressed_bytes[1..65];

    Ok(ethrex_crypto::keccak::keccak_hash(xy))
}

/// Transaction sender recovery using k256 (pure Rust, RISC-V compatible).
pub(crate) fn k256_recover_signer(
    sig: &[u8; 65],
    msg: &[u8; 32],
) -> Result<Address, CryptoError> {
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
    let signature =
        Signature::from_slice(&sig[..64]).map_err(|_| CryptoError::InvalidSignature)?;

    #[allow(clippy::indexing_slicing)]
    let recovery_id =
        RecoveryId::from_byte(sig[64]).ok_or(CryptoError::InvalidRecoveryId)?;

    let vk = VerifyingKey::recover_from_prehash(msg, &signature, recovery_id)
        .map_err(|_| CryptoError::RecoveryFailed)?;

    let uncompressed = vk.to_encoded_point(false);
    let uncompressed_bytes = uncompressed.as_bytes();
    #[allow(clippy::indexing_slicing)]
    let xy = &uncompressed_bytes[1..65];
    let hash = ethrex_crypto::keccak::keccak_hash(xy);

    #[allow(clippy::indexing_slicing)]
    Ok(Address::from_slice(&hash[12..]))
}

// ── Shared substrate-bn implementations ──────────────────────────────────

/// BN254 G1 scalar multiplication using substrate-bn (pure Rust, RISC-V compatible).
pub(crate) fn substrate_bn_g1_mul(
    point: &[u8],
    scalar: &[u8],
) -> Result<[u8; 64], CryptoError> {
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
    #[allow(clippy::indexing_slicing)]
    {
        result.x().to_big_endian(&mut out[..32]);
        result.y().to_big_endian(&mut out[32..]);
    }
    Ok(out)
}

/// BN254 pairing check using substrate-bn (pure Rust, RISC-V compatible).
pub(crate) fn substrate_bn_pairing_check(
    pairs: &[(&[u8], &[u8])],
) -> Result<bool, CryptoError> {
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
        let g1x =
            Fq::from_slice(&g1_bytes[..32]).map_err(|_| CryptoError::InvalidInput("G1.x"))?;
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
        let g2_x_re = Fq::from_slice(&g2_bytes[32..64])
            .map_err(|_| CryptoError::InvalidInput("G2.x_re"))?;
        #[allow(clippy::indexing_slicing)]
        let g2_y_im = Fq::from_slice(&g2_bytes[64..96])
            .map_err(|_| CryptoError::InvalidInput("G2.y_im"))?;
        #[allow(clippy::indexing_slicing)]
        let g2_y_re = Fq::from_slice(&g2_bytes[96..128])
            .map_err(|_| CryptoError::InvalidInput("G2.y_re"))?;

        let g2: G2 = if g2_x_im.is_zero()
            && g2_x_re.is_zero()
            && g2_y_im.is_zero()
            && g2_y_re.is_zero()
        {
            G2::zero()
        } else {
            AffineG2::new(Fq2::new(g2_x_im, g2_x_re), Fq2::new(g2_y_im, g2_y_re))
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
