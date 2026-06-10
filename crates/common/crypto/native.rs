use crate::provider::Crypto;

/// Native crypto implementation using system libraries.
///
/// Most method bodies live as defaults on the [`Crypto`] trait itself. The
/// P-256 (secp256r1) verify is overridden here to use the assembly-optimized
/// `aws-lc-rs` backend when the `aws-lc-rs` feature is enabled, since the
/// portable `p256` default does two constant-time scalar muls with no
/// Shamir/basepoint optimization and is a P256VERIFY hot-path outlier. This
/// struct exists so callers outside zkVM contexts have a concrete type to
/// instantiate.
#[derive(Debug)]
pub struct NativeCrypto;

#[cfg(not(feature = "aws-lc-rs"))]
impl Crypto for NativeCrypto {}

#[cfg(feature = "aws-lc-rs")]
impl Crypto for NativeCrypto {
    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        crate::p256_awslc::secp256r1_verify(msg, sig, pk)
    }
}
