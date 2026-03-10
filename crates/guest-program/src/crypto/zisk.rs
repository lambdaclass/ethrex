use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::shared::{
    k256_ecrecover, k256_recover_signer, substrate_bn_g1_add, substrate_bn_g1_mul,
    substrate_bn_pairing_check,
};

/// ZisK crypto provider.
///
/// Uses k256 for ECDSA (secp256k1) and substrate-bn for all BN254 operations.
/// Overrides `mulmod256` and `modexp` with ZisK's native circuit instructions via `ziskos`.
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

    /// ZisK-accelerated modular exponentiation via native circuit instruction.
    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let modulus_len = modulus.len();

        if modulus.iter().all(|&b| b == 0) {
            return Ok(vec![0u8; modulus_len]);
        }

        if exp.iter().all(|&b| b == 0) {
            // base^0 mod m = 1 mod m
            let mod_is_one =
                modulus.iter().rev().skip(1).all(|&b| b == 0) && modulus.last() == Some(&1);
            let mut result = vec![0u8; modulus_len];
            if !mod_is_one && modulus_len > 0 {
                #[allow(clippy::indexing_slicing)]
                {
                    result[modulus_len - 1] = 1;
                }
            }
            return Ok(result);
        }

        let base_limbs = bytes_be_to_limbs_asc(base);
        let exp_limbs = bytes_be_to_limbs_asc(exp);
        let modulus_limbs = bytes_be_to_limbs_asc(modulus);

        let result_limbs = ziskos::zisklib::modexp_u64(&base_limbs, &exp_limbs, &modulus_limbs);

        let result_bytes = limbs_asc_to_bytes_be(&result_limbs);

        let mut out = vec![0u8; modulus_len];
        #[allow(clippy::indexing_slicing)]
        if result_bytes.len() <= modulus_len {
            let offset = modulus_len - result_bytes.len();
            out[offset..].copy_from_slice(&result_bytes);
        } else {
            out.copy_from_slice(&result_bytes[result_bytes.len() - modulus_len..]);
        }
        Ok(out)
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

/// Convert big-endian bytes to u64 limbs in ascending order (least significant first).
/// This matches the limb layout expected by `ziskos::zisklib::modexp_u64` and is
/// equivalent to malachite's `Natural::to_limbs_asc()` on the same numeric value.
fn bytes_be_to_limbs_asc(bytes: &[u8]) -> Vec<u64> {
    if bytes.is_empty() || bytes.iter().all(|&b| b == 0) {
        return vec![0];
    }

    // Reverse to little-endian byte order
    let mut le_bytes: Vec<u8> = bytes.iter().rev().copied().collect();
    // Pad to multiple of 8
    while le_bytes.len() % 8 != 0 {
        le_bytes.push(0);
    }

    le_bytes
        .chunks_exact(8)
        .map(|chunk| {
            u64::from_le_bytes(
                chunk
                    .try_into()
                    .expect("chunks_exact(8) guarantees 8 bytes"),
            )
        })
        .collect()
}

/// Convert u64 limbs (ascending order, least significant first) to big-endian bytes.
fn limbs_asc_to_bytes_be(limbs: &[u64]) -> Vec<u8> {
    if limbs.is_empty() {
        return vec![0];
    }

    // Convert each limb to 8 BE bytes, starting from the highest limb
    let mut bytes = Vec::with_capacity(limbs.len() * 8);
    for &limb in limbs.iter().rev() {
        bytes.extend_from_slice(&limb.to_be_bytes());
    }

    // Strip leading zeros, keeping at least one byte
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(bytes.len().saturating_sub(1));
    bytes[start..].to_vec()
}
