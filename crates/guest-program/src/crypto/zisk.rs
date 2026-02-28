use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError, NativeCrypto};

use super::sp1::{
    k256_ecrecover, k256_recover_signer, substrate_bn_g1_mul, substrate_bn_pairing_check,
};

/// ZisK crypto provider.
///
/// Uses k256 for ECDSA (secp256k1) and substrate-bn for all BN254 operations.
/// Delegates all other operations to [`NativeCrypto`].
///
/// When building actual ZisK guest binaries, ZisK's patched crate versions
/// of k256 and substrate-bn are used transparently via Cargo patches.
/// ZisK-specific operations (modexp via ziskos FFI, mulmod256 via ziskos)
/// are not available on host builds and fall back to NativeCrypto.
#[derive(Debug)]
pub struct ZiskCrypto;

impl Crypto for ZiskCrypto {
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
        substrate_bn_g1_add(p1, p2)
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
        // ZisK guest binaries use ziskos FFI for modexp, but that only compiles
        // on the ZisK target. On host builds we fall back to NativeCrypto.
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

// ── ZisK-specific substrate-bn implementation ────────────────────────────

/// BN254 G1 point addition using substrate-bn (pure Rust, RISC-V compatible).
/// ZisK was the only target that used substrate-bn for ecadd (others use ark-bn254).
fn substrate_bn_g1_add(p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
    use substrate_bn::{AffineG1, Fq, G1, Group};

    if p1.len() < 64 {
        return Err(CryptoError::InvalidInput("P1 must be at least 64 bytes"));
    }
    if p2.len() < 64 {
        return Err(CryptoError::InvalidInput("P2 must be at least 64 bytes"));
    }

    // Parse P1
    #[allow(clippy::indexing_slicing)]
    let p1x =
        Fq::from_slice(&p1[..32]).map_err(|_| CryptoError::InvalidInput("invalid P1.x"))?;
    #[allow(clippy::indexing_slicing)]
    let p1y =
        Fq::from_slice(&p1[32..64]).map_err(|_| CryptoError::InvalidInput("invalid P1.y"))?;

    let g1_a: G1 = if p1x.is_zero() && p1y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(p1x, p1y)
            .map_err(|_| CryptoError::InvalidPoint("P1 not on BN254 curve"))?
            .into()
    };

    // Parse P2
    #[allow(clippy::indexing_slicing)]
    let p2x =
        Fq::from_slice(&p2[..32]).map_err(|_| CryptoError::InvalidInput("invalid P2.x"))?;
    #[allow(clippy::indexing_slicing)]
    let p2y =
        Fq::from_slice(&p2[32..64]).map_err(|_| CryptoError::InvalidInput("invalid P2.y"))?;

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
    #[allow(clippy::indexing_slicing)]
    {
        result.x().to_big_endian(&mut out[..32]);
        result.y().to_big_endian(&mut out[32..]);
    }
    Ok(out)
}
