use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::shared::{
    k256_ecrecover, k256_recover_signer, substrate_bn_g1_add, substrate_bn_g1_mul,
    substrate_bn_pairing_check,
};

/// Airbender crypto provider.
///
/// Uses shared pure-Rust crypto helpers (k256, substrate-bn) as a fallback.
/// These will be transparently accelerated by Airbender's CSR delegation
/// when the guest binary is compiled with the Airbender toolchain's
/// patched crate versions.
///
/// TODO: Replace with direct `airbender-crypto` delegation calls once
/// the API is confirmed. This would use CSR 0x7ca for bigint ops and
/// keccak_special5 for keccak, similar to ZisK v0.16.1's FFI pattern.
#[derive(Debug)]
pub struct AirbenderCrypto;

impl Crypto for AirbenderCrypto {
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

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        substrate_bn_g1_add(p1, p2)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        substrate_bn_g1_mul(point, scalar)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        substrate_bn_pairing_check(pairs)
    }
}
