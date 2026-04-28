//! 0x0100 — p256Verify (secp256r1 / P-256 signature verification)
//!
//! Equivalent to EIP-7212. BSC gas cost depends on fork:
//!   - Pre-Osaka:  3450  (BSC `P256VerifyGasBeforeOsaka`)
//!   - Post-Osaka: 6900  (BSC `params.P256VerifyGas`, EIP-7951)
//! Reference: `core/vm/contracts.go::p256Verify::RequiredGas` in bsc-geth.
//!
//! Input layout (160 bytes):
//! | message_hash (32) | r (32) | s (32) | pk_x (32) | pk_y (32) |
//!
//! Output: 32-byte big-endian `1` on success, empty slice on failure.

use super::PrecompileError;
use ethrex_common::types::Fork;
use p256::{
    EncodedPoint,
    ecdsa::{Signature as P256Signature, signature::hazmat::PrehashVerifier},
};

/// Pre-Osaka P256 verify gas cost (BSC `P256VerifyGasBeforeOsaka`).
pub const P256_VERIFY_GAS_PRE_OSAKA: u64 = 3450;
/// Post-Osaka (EIP-7951) P256 verify gas cost (BSC `params.P256VerifyGas`).
pub const P256_VERIFY_GAS_OSAKA: u64 = 6900;

#[inline]
pub fn p256_verify_gas(fork: Fork) -> u64 {
    if fork >= Fork::Osaka {
        P256_VERIFY_GAS_OSAKA
    } else {
        P256_VERIFY_GAS_PRE_OSAKA
    }
}

/// Input must be exactly this many bytes.
const INPUT_LENGTH: usize = 160;

/// 32-byte big-endian representation of the integer `1`.
const SUCCESS_RESULT: [u8; 32] = {
    let mut b = [0u8; 32];
    b[31] = 1;
    b
};

/// Run the p256Verify precompile.
///
/// Returns `(gas_used, output)` where `output` is:
/// - `SUCCESS_RESULT` (32 bytes) when the signature is valid.
/// - empty `Vec` when the signature is invalid (wrong length, bad point, etc.).
pub fn run(input: &[u8], gas_limit: u64, fork: Fork) -> Result<(u64, Vec<u8>), PrecompileError> {
    let gas_cost = p256_verify_gas(fork);
    if gas_limit < gas_cost {
        return Err(PrecompileError::NotEnoughGas);
    }

    // Wrong input length → return empty (not a hard revert).
    if input.len() != INPUT_LENGTH {
        return Ok((gas_cost, vec![]));
    }

    // Safety: length is checked above.
    let msg: &[u8; 32] = input[0..32].try_into().expect("length checked");
    let r: &[u8; 32] = input[32..64].try_into().expect("length checked");
    let s: &[u8; 32] = input[64..96].try_into().expect("length checked");
    let pk_x: &[u8; 32] = input[96..128].try_into().expect("length checked");
    let pk_y: &[u8; 32] = input[128..160].try_into().expect("length checked");

    let success = verify_p256(msg, r, s, pk_x, pk_y);
    if success {
        Ok((gas_cost, SUCCESS_RESULT.to_vec()))
    } else {
        Ok((gas_cost, vec![]))
    }
}

/// Verify a P-256 (secp256r1) signature using the `p256` crate.
///
/// Returns `true` iff the signature `(r, s)` over `msg_hash` is valid for the
/// public key `(pk_x, pk_y)`.  Any parsing failure is treated as invalid.
fn verify_p256(
    msg_hash: &[u8; 32],
    r: &[u8; 32],
    s: &[u8; 32],
    pk_x: &[u8; 32],
    pk_y: &[u8; 32],
) -> bool {
    // Build 64-byte signature (r || s).
    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r);
    sig_bytes[32..].copy_from_slice(s);

    let Ok(signature) = P256Signature::try_from(sig_bytes.as_slice()) else {
        return false;
    };

    // Build uncompressed public key from affine coordinates.
    let x_field: p256::FieldBytes = (*pk_x).into();
    let y_field: p256::FieldBytes = (*pk_y).into();
    let encoded = EncodedPoint::from_affine_coordinates(&x_field, &y_field, false);

    let Ok(verifying_key) = p256::ecdsa::VerifyingKey::from_encoded_point(&encoded) else {
        return false;
    };

    verifying_key.verify_prehash(msg_hash, &signature).is_ok()
}
