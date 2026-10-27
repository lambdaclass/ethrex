use crate::provider::{Crypto, CryptoError};

/// Native crypto implementation using system libraries.
///
/// Most method bodies live as defaults on the [`Crypto`] trait itself. The
/// P-256 (secp256r1) verify is overridden here to use the assembly-optimized
/// `aws-lc-rs` backend when the `aws-lc-rs` feature is enabled, since the
/// portable `p256` default does two constant-time scalar muls with no
/// Shamir/basepoint optimization and is a P256VERIFY hot-path outlier. On the
/// host the BLS12-381 (EIP-2537) defaults likewise route through the
/// assembly-optimized `blst` backend (the `blst` feature, default-on); zkVM
/// guest builds compile both backends out and use their own `Crypto` providers
/// instead of this type. BN254 ECADD/ECMUL use the dedicated G1 code in
/// [`crate::bn254`], which avoids the generic decoding, inversion and scalar
/// multiplication paths of the trait defaults. This
/// struct exists so callers outside zkVM contexts have a concrete type to
/// instantiate.
#[derive(Debug)]
pub struct NativeCrypto;

impl Crypto for NativeCrypto {
    #[cfg(feature = "aws-lc-rs")]
    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        crate::p256_awslc::secp256r1_verify(msg, sig, pk)
    }

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        crate::bn254::g1_add(p1, p2)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        crate::bn254::g1_mul(point, scalar)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        crate::bn254::pairing_check(pairs)
    }
}
