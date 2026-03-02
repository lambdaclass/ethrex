use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::sp1::{
    k256_ecrecover, k256_recover_signer, substrate_bn_g1_mul, substrate_bn_pairing_check,
};

/// ZisK crypto provider.
///
/// Uses k256 for ECDSA (secp256k1) and substrate-bn for all BN254 operations.
/// All other operations use the trait defaults (native libraries).
///
/// When building actual ZisK guest binaries, ZisK's patched crate versions
/// of k256 and substrate-bn are used transparently via Cargo patches.
/// ZisK-specific operations (modexp via ziskos FFI, mulmod256 via ziskos)
/// are not available on host builds and fall back to trait defaults.
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
}

// ── ZisK-specific substrate-bn implementation ────────────────────────────

/// BN254 G1 point addition using substrate-bn (pure Rust, RISC-V compatible).
/// ZisK was the only target that used substrate-bn for ecadd (others use ark-bn254).
fn substrate_bn_g1_add(p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
    use substrate_bn::{AffineG1, Fq, G1, Group};

    if p1.len() < 64 {
        return Err(CryptoError::InvalidInput("P1 must be at least 64 bytes"));
    }
    if p2.len() < 64 {
        return Err(CryptoError::InvalidInput("P2 must be at least 64 bytes"));
    }

    // Parse P1
    #[allow(clippy::indexing_slicing)]
    let p1x =
        Fq::from_slice(&p1[..32]).map_err(|_| CryptoError::InvalidInput("invalid P1.x"))?;
    #[allow(clippy::indexing_slicing)]
    let p1y =
        Fq::from_slice(&p1[32..64]).map_err(|_| CryptoError::InvalidInput("invalid P1.y"))?;

    let g1_a: G1 = if p1x.is_zero() && p1y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(p1x, p1y)
            .map_err(|_| CryptoError::InvalidPoint("P1 not on BN254 curve"))?
            .into()
    };

    // Parse P2
    #[allow(clippy::indexing_slicing)]
    let p2x =
        Fq::from_slice(&p2[..32]).map_err(|_| CryptoError::InvalidInput("invalid P2.x"))?;
    #[allow(clippy::indexing_slicing)]
    let p2y =
        Fq::from_slice(&p2[32..64]).map_err(|_| CryptoError::InvalidInput("invalid P2.y"))?;

    let g1_b: G1 = if p2x.is_zero() && p2y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(p2x, p2y)
            .map_err(|_| CryptoError::InvalidPoint("P2 not on BN254 curve"))?
            .into()
    };

    #[allow(clippy::arithmetic_side_effects)]
    let result = g1_a + g1_b;

    let mut out = [0u8; 64];
    #[allow(clippy::indexing_slicing)]
    {
        result.x().to_big_endian(&mut out[..32]);
        result.y().to_big_endian(&mut out[32..]);
    }
    Ok(out)
}
