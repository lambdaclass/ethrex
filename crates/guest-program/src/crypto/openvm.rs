use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError, NativeCrypto};

use super::sp1::{k256_ecrecover, k256_recover_signer};

/// OpenVM crypto provider.
///
/// Uses k256 for ECDSA (secp256k1).
/// Delegates all other operations to [`NativeCrypto`].
///
/// When building actual OpenVM guest binaries, OpenVM's patched crate version
/// of k256 is used transparently via Cargo patches.
#[derive(Debug)]
pub struct OpenVmCrypto;

impl Crypto for OpenVmCrypto {
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
        NativeCrypto.bn254_g1_mul(point, scalar)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        NativeCrypto.bn254_pairing_check(pairs)
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
