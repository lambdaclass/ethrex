use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::sp1::{k256_ecrecover, k256_recover_signer};

/// OpenVM crypto provider.
///
/// Uses k256 for ECDSA (secp256k1).
/// All other operations use the trait defaults (native libraries).
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
}
