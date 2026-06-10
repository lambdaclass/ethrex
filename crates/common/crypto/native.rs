use crate::provider::Crypto;

/// Native crypto implementation using system libraries.
///
/// Most method bodies live as defaults on the [`Crypto`] trait itself. Two
/// hot-path operations are overridden here to use assembly-optimized backends
/// when their (default-on, native-only) features are enabled:
///
/// - BLS12-381 (EIP-2537) ops through `blst` (`blst` feature), since the
///   portable `bls12_381` defaults are markedly slower on the cheap
///   G1ADD/G2ADD precompiles.
/// - P-256 (secp256r1) verify through `aws-lc-rs` (`aws-lc-rs` feature), since
///   the portable `p256` default does two constant-time scalar muls with no
///   Shamir/basepoint optimization, making P256VERIFY an outlier.
///
/// Each override is gated per-method so the backends compose independently;
/// zkVM guest builds omit both features and use the portable trait defaults.
/// This struct exists so callers outside zkVM contexts have a concrete type to
/// instantiate.
#[derive(Debug)]
pub struct NativeCrypto;

impl Crypto for NativeCrypto {
    #[cfg(feature = "aws-lc-rs")]
    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        crate::p256_awslc::secp256r1_verify(msg, sig, pk)
    }

    #[cfg(feature = "blst")]
    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], crate::CryptoError> {
        crate::bls_blst::g1_add(a, b)
    }

    #[cfg(feature = "blst")]
    #[allow(clippy::type_complexity)]
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], crate::CryptoError> {
        crate::bls_blst::g1_msm(pairs)
    }

    #[cfg(feature = "blst")]
    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], crate::CryptoError> {
        crate::bls_blst::g2_add(a, b)
    }

    #[cfg(feature = "blst")]
    #[allow(clippy::type_complexity)]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], crate::CryptoError> {
        crate::bls_blst::g2_msm(pairs)
    }

    #[cfg(feature = "blst")]
    #[allow(clippy::type_complexity)]
    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, crate::CryptoError> {
        crate::bls_blst::pairing_check(pairs)
    }

    #[cfg(feature = "blst")]
    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], crate::CryptoError> {
        crate::bls_blst::fp_to_g1(fp)
    }

    #[cfg(feature = "blst")]
    fn bls12_381_fp2_to_g2(
        &self,
        fp2: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], crate::CryptoError> {
        crate::bls_blst::fp2_to_g2(fp2)
    }
}
