use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

#[derive(Debug)]
pub struct ZiskCrypto;

impl Crypto for ZiskCrypto {
    fn secp256k1_ecrecover(
        &self,
        _sig: &[u8; 64],
        _recid: u8,
        _msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        todo!("ZisK secp256k1_ecrecover implementation")
    }

    fn recover_signer(&self, _sig: &[u8; 65], _msg: &[u8; 32]) -> Result<Address, CryptoError> {
        todo!("ZisK recover_signer implementation")
    }

    fn sha256(&self, _input: &[u8]) -> [u8; 32] {
        todo!("ZisK sha256 implementation")
    }

    fn ripemd160(&self, _input: &[u8]) -> [u8; 32] {
        todo!("ZisK ripemd160 implementation")
    }

    fn bn254_g1_add(&self, _p1: &[u8], _p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        todo!("ZisK bn254_g1_add implementation")
    }

    fn bn254_g1_mul(&self, _point: &[u8], _scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        todo!("ZisK bn254_g1_mul implementation")
    }

    fn bn254_pairing_check(&self, _pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        todo!("ZisK bn254_pairing_check implementation")
    }

    fn modexp(
        &self,
        _base: &[u8],
        _exp: &[u8],
        _modulus: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        todo!("ZisK modexp implementation")
    }

    fn blake2_compress(
        &self,
        _rounds: u32,
        _h: &mut [u64; 8],
        _m: [u64; 16],
        _t: [u64; 2],
        _f: bool,
    ) {
        todo!("ZisK blake2_compress implementation")
    }

    fn secp256r1_verify(&self, _msg: &[u8; 32], _sig: &[u8; 64], _pk: &[u8; 64]) -> bool {
        todo!("ZisK secp256r1_verify implementation")
    }

    fn verify_kzg_proof(
        &self,
        _z: &[u8; 32],
        _y: &[u8; 32],
        _commitment: &[u8; 48],
        _proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        todo!("ZisK verify_kzg_proof implementation")
    }

    fn verify_blob_kzg_proof(
        &self,
        _blob: &[u8],
        _commitment: &[u8; 48],
        _proof: &[u8; 48],
    ) -> Result<bool, CryptoError> {
        todo!("ZisK verify_blob_kzg_proof implementation")
    }

    fn bls12_381_g1_add(
        &self,
        _a: ([u8; 48], [u8; 48]),
        _b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        todo!("ZisK bls12_381_g1_add implementation")
    }

    fn bls12_381_g1_msm(
        &self,
        _pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        todo!("ZisK bls12_381_g1_msm implementation")
    }

    fn bls12_381_g2_add(
        &self,
        _a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        _b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        todo!("ZisK bls12_381_g2_add implementation")
    }

    fn bls12_381_g2_msm(
        &self,
        _pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        todo!("ZisK bls12_381_g2_msm implementation")
    }

    fn bls12_381_pairing_check(
        &self,
        _pairs: &[(([u8; 48], [u8; 48]), ([u8; 48], [u8; 48], [u8; 48], [u8; 48]))],
    ) -> Result<bool, CryptoError> {
        todo!("ZisK bls12_381_pairing_check implementation")
    }

    fn bls12_381_fp_to_g1(&self, _fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        todo!("ZisK bls12_381_fp_to_g1 implementation")
    }

    fn bls12_381_fp2_to_g2(&self, _fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        todo!("ZisK bls12_381_fp2_to_g2 implementation")
    }
}
