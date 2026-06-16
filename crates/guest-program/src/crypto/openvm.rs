use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::bls_portable::impl_portable_bls12_381;
use super::shared::{k256_ecrecover, k256_recover_signer};

/// OpenVM crypto provider.
///
/// Uses k256 for ECDSA (secp256k1), the portable `bls12_381` backend for
/// BLS12-381 (EIP-2537), and `openvm-kzg` for the KZG point-evaluation
/// precompile. All other operations use the trait defaults (native libraries).
///
/// When building actual OpenVM guest binaries, OpenVM's patched crate version
/// of k256 is used transparently via Cargo patches, and `openvm-kzg` uses
/// OpenVM intrinsics for the KZG verification.
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

    /// EIP-4844 point-evaluation precompile, accelerated with OpenVM intrinsics
    /// via `openvm-kzg`. Relocated here from `ethrex-crypto` so the published
    /// crate carries no `openvm-kzg` git dependency.
    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        let map_err = |e| CryptoError::Other(format!("openvm-kzg: {e}"));
        let valid = openvm_kzg::KzgProof::verify_kzg_proof(
            &openvm_kzg::Bytes48::from_slice(commitment).map_err(map_err)?,
            &openvm_kzg::Bytes32::from_slice(z).map_err(map_err)?,
            &openvm_kzg::Bytes32::from_slice(y).map_err(map_err)?,
            &openvm_kzg::Bytes48::from_slice(proof).map_err(map_err)?,
            &openvm_kzg::get_kzg_settings(),
        )
        .map_err(map_err)?;
        if valid {
            Ok(())
        } else {
            Err(CryptoError::VerificationFailed)
        }
    }

    impl_portable_bls12_381!();
}
