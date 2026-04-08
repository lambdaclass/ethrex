//! 0x0100 — p256Verify (secp256r1 / P-256 signature verification)
//!
//! Equivalent to EIP-7212.  BSC gas cost: 3450 (see params.P256VerifyGas in
//! the BSC reference implementation core/vm/contracts.go).
//!
//! Input layout (160 bytes):
//! | message_hash (32) | r (32) | s (32) | pk_x (32) | pk_y (32) |
//!
//! Output: 32-byte big-endian `1` on success, empty slice on failure.

use super::PrecompileError;
use p256::{
    EncodedPoint,
    ecdsa::{Signature as P256Signature, signature::hazmat::PrehashVerifier},
};

/// Gas cost for p256Verify.  Matches `params.P256VerifyGas` in the BSC
/// reference implementation (`core/vm/contracts.go`).
pub const P256_VERIFY_GAS: u64 = 3450;

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
pub fn run(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if gas_limit < P256_VERIFY_GAS {
        return Err(PrecompileError::NotEnoughGas);
    }

    // Wrong input length → return empty (not a hard revert).
    if input.len() != INPUT_LENGTH {
        return Ok((P256_VERIFY_GAS, vec![]));
    }

    // Safety: length is checked above.
    let msg: &[u8; 32] = input[0..32].try_into().expect("length checked");
    let r: &[u8; 32] = input[32..64].try_into().expect("length checked");
    let s: &[u8; 32] = input[64..96].try_into().expect("length checked");
    let pk_x: &[u8; 32] = input[96..128].try_into().expect("length checked");
    let pk_y: &[u8; 32] = input[128..160].try_into().expect("length checked");

    let success = verify_p256(msg, r, s, pk_x, pk_y);
    if success {
        Ok((P256_VERIFY_GAS, SUCCESS_RESULT.to_vec()))
    } else {
        Ok((P256_VERIFY_GAS, vec![]))
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
