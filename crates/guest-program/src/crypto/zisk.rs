use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::shared::{
    k256_ecrecover, k256_recover_signer, substrate_bn_g1_add, substrate_bn_g1_mul,
    substrate_bn_pairing_check,
};

/// ZisK crypto provider.
///
/// Uses k256 for ECDSA (secp256k1) and substrate-bn for all BN254 operations.
/// Overrides `mulmod256` with ZisK's native circuit instruction via `ziskos`.
///
/// When building actual ZisK guest binaries, ZisK's patched crate versions
/// of k256 and substrate-bn are used transparently via Cargo patches.
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

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        substrate_bn_g1_add(p1, p2)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        substrate_bn_g1_mul(point, scalar)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        substrate_bn_pairing_check(pairs)
    }

    /// ZisK-accelerated 256-bit modular multiplication via native circuit instruction.
    fn mulmod256(&self, a: &[u8; 32], b: &[u8; 32], m: &[u8; 32]) -> [u8; 32] {
        use ethereum_types::U256;

        let m_u256 = U256::from_big_endian(m);
        if m_u256.is_zero() {
            return [0u8; 32];
        }

        let a_u256 = U256::from_big_endian(a);
        let b_u256 = U256::from_big_endian(b);

        let mut result = U256::zero();
        // SAFETY: ziskos FFI is safe when called with valid 4-element u64 arrays.
        // U256::0 is [u64; 4] in little-endian word order, matching ziskos ABI.
        unsafe {
            ziskos::zisklib::mulmod256_c(
                a_u256.0.as_ptr(),
                b_u256.0.as_ptr(),
                m_u256.0.as_ptr(),
                result.0.as_mut_ptr(),
            );
        }
        result.to_big_endian()
    }
}
