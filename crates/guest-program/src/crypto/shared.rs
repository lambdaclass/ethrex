/// Shared crypto helper functions used by multiple zkVM providers.
///
/// These functions are implemented in pure Rust using crates that are
/// patched by each zkVM toolchain (k256, substrate-bn) to use their
/// respective circuit accelerators transparently via Cargo patches.
use ethereum_types::Address;
use ethrex_crypto::{CryptoError, keccak::keccak_hash};

// ── k256 ECDSA ───────────────────────────────────────────────────────────────

/// ECDSA public key recovery using k256 (pure Rust, RISC-V compatible).
/// Used by the ECRECOVER precompile (0x01).
pub(crate) fn k256_ecrecover(
    sig: &[u8; 64],
    recid: u8,
    msg: &[u8; 32],
) -> Result<[u8; 32], CryptoError> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let mut sig_obj = Signature::from_slice(sig).map_err(|_| CryptoError::InvalidSignature)?;

    let mut recid_byte = recid;
    // k256 enforces canonical low-S for recovery.
    // If S is high, normalize s := n - s and flip the recovery parity bit.
    if let Some(low_s) = sig_obj.normalize_s() {
        sig_obj = low_s;
        recid_byte ^= 1;
    }

    let recovery_id = RecoveryId::from_byte(recid_byte).ok_or(CryptoError::InvalidRecoveryId)?;

    let vk = VerifyingKey::recover_from_prehash(msg, &sig_obj, recovery_id)
        .map_err(|_| CryptoError::RecoveryFailed)?;

    // SEC1 uncompressed: 0x04 || X(32) || Y(32). We need keccak(X||Y).
    let uncompressed = vk.to_encoded_point(false);
    let uncompressed_bytes = uncompressed.as_bytes();
    #[allow(clippy::indexing_slicing)]
    Ok(keccak_hash(&uncompressed_bytes[1..65]))
}

/// Transaction sender recovery using k256 (pure Rust, RISC-V compatible).
/// Used by tx.sender() and EIP-7702 authority recovery.
pub(crate) fn k256_recover_signer(sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    // EIP-2: reject high-s signatures (s > secp256k1n/2)
    const SECP256K1_N_HALF: [u8; 32] = [
        0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b,
        0x20, 0xa0,
    ];
    #[allow(clippy::indexing_slicing)]
    if sig[32..64] > SECP256K1_N_HALF[..] {
        return Err(CryptoError::InvalidSignature);
    }

    #[allow(clippy::indexing_slicing)]
    let signature = Signature::from_slice(&sig[..64]).map_err(|_| CryptoError::InvalidSignature)?;

    #[allow(clippy::indexing_slicing)]
    let recovery_id = RecoveryId::from_byte(sig[64]).ok_or(CryptoError::InvalidRecoveryId)?;

    let vk = VerifyingKey::recover_from_prehash(msg, &signature, recovery_id)
        .map_err(|_| CryptoError::RecoveryFailed)?;

    let uncompressed = vk.to_encoded_point(false);
    let uncompressed_bytes = uncompressed.as_bytes();
    #[allow(clippy::indexing_slicing)]
    let hash = keccak_hash(&uncompressed_bytes[1..65]);

    #[allow(clippy::indexing_slicing)]
    Ok(Address::from_slice(&hash[12..]))
}

// ── substrate-bn BN254 ───────────────────────────────────────────────────────
//
// These functions require the substrate-bn crate, which is only available when
// sp1, risc0, or zisk is enabled. The openvm feature only provides k256 and
// does not need BN254 via substrate-bn.
#[cfg(any(feature = "sp1", feature = "risc0", feature = "zisk"))]
/// BN254 G1 point addition using substrate-bn (pure Rust, RISC-V compatible).
/// Used by ZisK, which historically used substrate-bn for ecadd rather than ark-bn254.
pub(crate) fn substrate_bn_g1_add(p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
    use substrate_bn::{AffineG1, Fq, G1, Group};

    if p1.len() < 64 {
        return Err(CryptoError::InvalidInput("P1 must be at least 64 bytes"));
    }
    if p2.len() < 64 {
        return Err(CryptoError::InvalidInput("P2 must be at least 64 bytes"));
    }

    #[allow(clippy::indexing_slicing)]
    let p1x = Fq::from_slice(&p1[..32]).map_err(|_| CryptoError::InvalidInput("invalid P1.x"))?;
    #[allow(clippy::indexing_slicing)]
    let p1y = Fq::from_slice(&p1[32..64]).map_err(|_| CryptoError::InvalidInput("invalid P1.y"))?;

    let g1_a: G1 = if p1x.is_zero() && p1y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(p1x, p1y)
            .map_err(|_| CryptoError::InvalidPoint("P1 not on BN254 curve"))?
            .into()
    };

    #[allow(clippy::indexing_slicing)]
    let p2x = Fq::from_slice(&p2[..32]).map_err(|_| CryptoError::InvalidInput("invalid P2.x"))?;
    #[allow(clippy::indexing_slicing)]
    let p2y = Fq::from_slice(&p2[32..64]).map_err(|_| CryptoError::InvalidInput("invalid P2.y"))?;

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
    if let Some(affine) = AffineG1::from_jacobian(result) {
        #[allow(clippy::indexing_slicing)]
        affine.x().to_big_endian(&mut out[..32]);
        #[allow(clippy::indexing_slicing)]
        affine.y().to_big_endian(&mut out[32..]);
    }
    Ok(out)
}

#[cfg(any(feature = "sp1", feature = "risc0", feature = "zisk"))]
/// BN254 G1 scalar multiplication using substrate-bn (pure Rust, RISC-V compatible).
/// Used by SP1 and ZisK where substrate-bn is patched for circuit acceleration.
pub(crate) fn substrate_bn_g1_mul(point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
    use substrate_bn::{AffineG1, Fq, Fr, G1, Group};

    if point.len() < 64 || scalar.len() < 32 {
        return Err(CryptoError::InvalidInput("invalid input length"));
    }

    #[allow(clippy::indexing_slicing)]
    let x = Fq::from_slice(&point[..32]).map_err(|_| CryptoError::InvalidInput("invalid Fq x"))?;
    #[allow(clippy::indexing_slicing)]
    let y =
        Fq::from_slice(&point[32..64]).map_err(|_| CryptoError::InvalidInput("invalid Fq y"))?;

    if x.is_zero() && y.is_zero() {
        return Ok([0u8; 64]);
    }

    let g1: G1 = AffineG1::new(x, y)
        .map_err(|_| CryptoError::InvalidPoint("G1 not on BN254 curve"))?
        .into();

    #[allow(clippy::indexing_slicing)]
    let s = Fr::from_slice(&scalar[..32]).map_err(|_| CryptoError::InvalidInput("invalid Fr"))?;

    if s.is_zero() {
        return Ok([0u8; 64]);
    }

    #[allow(clippy::arithmetic_side_effects)]
    let result = g1 * s;

    let mut out = [0u8; 64];
    if let Some(affine) = AffineG1::from_jacobian(result) {
        #[allow(clippy::indexing_slicing)]
        affine.x().to_big_endian(&mut out[..32]);
        #[allow(clippy::indexing_slicing)]
        affine.y().to_big_endian(&mut out[32..]);
    }
    Ok(out)
}

#[cfg(any(feature = "sp1", feature = "risc0", feature = "zisk"))]
/// BN254 pairing check using substrate-bn (pure Rust, RISC-V compatible).
/// Used by SP1, RISC0, and ZisK where substrate-bn is patched for circuit acceleration.
pub(crate) fn substrate_bn_pairing_check(pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
    use substrate_bn::{AffineG1, AffineG2, Fq, Fq2, G1, G2, Group};

    if pairs.is_empty() {
        return Ok(true);
    }

    let mut valid_batch = Vec::with_capacity(pairs.len());

    for (g1_bytes, g2_bytes) in pairs {
        if g1_bytes.len() < 64 {
            return Err(CryptoError::InvalidInput("G1 must be 64 bytes"));
        }
        if g2_bytes.len() < 128 {
            return Err(CryptoError::InvalidInput("G2 must be 128 bytes"));
        }

        // Parse G1
        #[allow(clippy::indexing_slicing)]
        let g1x = Fq::from_slice(&g1_bytes[..32]).map_err(|_| CryptoError::InvalidInput("G1.x"))?;
        #[allow(clippy::indexing_slicing)]
        let g1y =
            Fq::from_slice(&g1_bytes[32..64]).map_err(|_| CryptoError::InvalidInput("G1.y"))?;

        let g1: G1 = if g1x.is_zero() && g1y.is_zero() {
            G1::zero()
        } else {
            AffineG1::new(g1x, g1y)
                .map_err(|_| CryptoError::InvalidPoint("G1 not on curve"))?
                .into()
        };

        // Parse G2 — EVM encodes as (x_im, x_re, y_im, y_re) each 32 bytes
        #[allow(clippy::indexing_slicing)]
        let g2_x_im =
            Fq::from_slice(&g2_bytes[..32]).map_err(|_| CryptoError::InvalidInput("G2.x_im"))?;
        #[allow(clippy::indexing_slicing)]
        let g2_x_re =
            Fq::from_slice(&g2_bytes[32..64]).map_err(|_| CryptoError::InvalidInput("G2.x_re"))?;
        #[allow(clippy::indexing_slicing)]
        let g2_y_im =
            Fq::from_slice(&g2_bytes[64..96]).map_err(|_| CryptoError::InvalidInput("G2.y_im"))?;
        #[allow(clippy::indexing_slicing)]
        let g2_y_re =
            Fq::from_slice(&g2_bytes[96..128]).map_err(|_| CryptoError::InvalidInput("G2.y_re"))?;

        let g2: G2 =
            if g2_x_im.is_zero() && g2_x_re.is_zero() && g2_y_im.is_zero() && g2_y_re.is_zero() {
                G2::zero()
            } else {
                AffineG2::new(Fq2::new(g2_x_re, g2_x_im), Fq2::new(g2_y_re, g2_y_im))
                    .map_err(|_| CryptoError::InvalidPoint("G2 not on curve"))?
                    .into()
            };

        if g1.is_zero() || g2.is_zero() {
            continue;
        }
        valid_batch.push((g1, g2));
    }

    let result = substrate_bn::pairing_batch(&valid_batch);
    Ok(result == substrate_bn::Gt::one())
}
