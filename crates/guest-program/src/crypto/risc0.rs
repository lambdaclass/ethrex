use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::shared::{k256_ecrecover, k256_recover_signer, substrate_bn_pairing_check};

/// RISC0 crypto provider.
///
/// Uses k256 for ECDSA (secp256k1) and substrate-bn for BN254 pairing.
/// All other operations use the trait defaults (native libraries).
///
/// When building actual RISC0 guest binaries, RISC0's patched crate versions
/// of k256 and substrate-bn are used transparently via Cargo patches.
#[derive(Debug)]
pub struct Risc0Crypto;

impl Crypto for Risc0Crypto {
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

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        substrate_bn_pairing_check(pairs)
    }
}
