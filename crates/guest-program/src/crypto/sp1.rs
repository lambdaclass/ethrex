use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::shared::{
    k256_ecrecover, k256_recover_signer, substrate_bn_g1_mul, substrate_bn_pairing_check,
};

/// SP1 crypto provider.
///
/// Uses k256 for ECDSA (secp256k1) and substrate-bn for BN254 ecmul/pairing.
/// All other operations use the trait defaults (native libraries).
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

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        substrate_bn_g1_mul(point, scalar)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        substrate_bn_pairing_check(pairs)
    }

    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        ethrex_crypto::kzg::verify_kzg_proof(*commitment, *z, *y, *proof)
            .map_err(|e| CryptoError::Other(format!("{e}")))
            .and_then(|valid| {
                if valid {
                    Ok(())
                } else {
                    Err(CryptoError::VerificationFailed)
                }
            })
    }
}
