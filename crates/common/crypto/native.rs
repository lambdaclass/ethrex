use crate::provider::Crypto;

/// Native crypto implementation using system libraries.
///
/// Most method bodies live as defaults on the [`Crypto`] trait itself. The
/// BLS12-381 (EIP-2537) operations are overridden here to use the assembly
/// optimized `blst` backend when the `blst` feature is enabled, since the
/// portable `bls12_381` defaults are markedly slower on the cheap G1ADD/G2ADD
/// precompiles. This struct exists so callers outside zkVM contexts have a
/// concrete type to instantiate.
#[derive(Debug)]
pub struct NativeCrypto;

#[cfg(not(feature = "blst"))]
impl Crypto for NativeCrypto {}

#[cfg(feature = "blst")]
impl Crypto for NativeCrypto {
    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], crate::CryptoError> {
        crate::bls_blst::g1_add(a, b)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], crate::CryptoError> {
        crate::bls_blst::g1_msm(pairs)
    }

    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], crate::CryptoError> {
        crate::bls_blst::g2_add(a, b)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], crate::CryptoError> {
        crate::bls_blst::g2_msm(pairs)
    }

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

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], crate::CryptoError> {
        crate::bls_blst::fp_to_g1(fp)
    }

    fn bls12_381_fp2_to_g2(
        &self,
        fp2: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], crate::CryptoError> {
        crate::bls_blst::fp2_to_g2(fp2)
    }
}
