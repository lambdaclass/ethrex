use bytes::Bytes;
use ethrex_common::{Address, H160, types::Fork};
use p256::{
    EncodedPoint, FieldElement as P256FieldElement, NistP256,
    ecdsa::{Signature as P256Signature, VerifyingKey, signature::hazmat::PrehashVerifier},
    elliptic_curve::{Curve, bigint::U256 as P256Uint, ff::PrimeField},
};
use std::sync::LazyLock;

use crate::{
    errors::{InternalError, PrecompileError, VMError},
    precompiles::{self},
};

pub const P256VERIFY_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x01, 0x00,
]);

pub const RIP_PRECOMPILES: [H160; 1] = [P256VERIFY_ADDRESS];

// Secp256r1 curve parameters
// See https://neuromancer.sk/std/secg/secp256r1
const P256_P: P256Uint = P256Uint::from_be_hex(P256FieldElement::MODULUS);
const P256_N: P256Uint = NistP256::ORDER;
const P256_A: P256FieldElement = P256FieldElement::from_u64(3).neg();
const P256_B_UINT: P256Uint =
    P256Uint::from_be_hex("5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b");
static P256_B: LazyLock<P256FieldElement> = LazyLock::new(|| P256FieldElement::from_uint(P256_B_UINT).unwrap());

pub const P256VERIFY_COST: u64 = 3450;

pub fn execute_precompile(
    address: Address,
    calldata: &Bytes,
    gas_remaining: &mut u64,
    fork: Fork,
) -> Result<Bytes, VMError> {
    let result = match address {
        address if address == P256VERIFY_ADDRESS => p_256_verify(calldata, gas_remaining)?,
        _ => return precompiles::execute_precompile(address, calldata, gas_remaining, fork),
    };
    Ok(result)
}

pub fn is_precompile(address: &Address, fork: Fork) -> bool {
    precompiles::is_precompile(address, fork) || RIP_PRECOMPILES.contains(address)
}

/// Signature verification in the “secp256r1” elliptic curve
/// If the verification succeeds, returns 1 in a 32-bit big-endian format.
/// If the verification fails, returns an empty `Bytes` object.
/// Implemented following https://github.com/ethereum/RIPs/blob/89474e2b9dbd066fac9446c8cd280651bda35849/RIPS/rip-7212.md?plain=1#L1.
pub fn p_256_verify(calldata: &Bytes, gas_remaining: &mut u64) -> Result<Bytes, VMError> {
    let gas_cost = P256VERIFY_COST;
    precompiles::increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = precompiles::fill_with_zeros(calldata, 160);

    // Parse parameters
    let message_hash = calldata
        .get(0..32)
        .ok_or(PrecompileError::ParsingInputError)?;
    let r = calldata
        .get(32..64)
        .ok_or(PrecompileError::ParsingInputError)?;
    let s = calldata
        .get(64..96)
        .ok_or(PrecompileError::ParsingInputError)?;
    let x = calldata
        .get(96..128)
        .ok_or(PrecompileError::ParsingInputError)?;
    let y = calldata
        .get(128..160)
        .ok_or(PrecompileError::ParsingInputError)?;

    if !validate_p256_parameters(r, s, x, y)? {
        return Ok(Bytes::new());
    }

    // Build verifier
    let Ok(verifier) = VerifyingKey::from_encoded_point(&EncodedPoint::from_affine_coordinates(
        x.into(),
        y.into(),
        false,
    )) else {
        return Ok(Bytes::new());
    };

    // Build signature
    let r: [u8; 32] = r.try_into().map_err(|_| InternalError::Slicing)?;
    let s: [u8; 32] = s.try_into().map_err(|_| InternalError::Slicing)?;

    let Ok(signature) = P256Signature::from_scalars(r, s) else {
        return Ok(Bytes::new());
    };

    // Verify message signature
    let success = verifier.verify_prehash(message_hash, &signature).is_ok();

    // If the verification succeeds, returns 1 in a 32-bit big-endian format.
    // If the verification fails, returns an empty `Bytes` object.
    if success {
        let mut result = [0; 32];
        result[31] = 1;
        Ok(Bytes::from(result.to_vec()))
    } else {
        Ok(Bytes::new())
    }
}

/// Following https://github.com/ethereum/RIPs/blob/89474e2b9dbd066fac9446c8cd280651bda35849/RIPS/rip-7212.md?plain=1#L86
fn validate_p256_parameters(r: &[u8], s: &[u8], x: &[u8], y: &[u8]) -> Result<bool, VMError> {
    let [r, s, x, y] = [r, s, x, y].map(P256Uint::from_be_slice);

    // Verify that the r and s values are in (0, n) (exclusive)
    if r == P256Uint::ZERO || r >= P256_N || s == P256Uint::ZERO || s >= P256_N {
        return Ok(false);
    }

    // Verify that both x and y are in [0, p) (inclusive 0, exclusive p)
    if x >= P256_P || y >= P256_P {
        return Ok(false);
    }

    // Verify that the point formed by (x, y) is on the curve
    let x: Option<P256FieldElement> = P256FieldElement::from_uint(x).into();
    let y: Option<P256FieldElement> = P256FieldElement::from_uint(y).into();

    let (Some(x), Some(y)) = (x, y) else {
        return Err(InternalError::Slicing.into());
    };

    // Curve equation: `y² = x³ + ax + b`
    let a_x = P256_A.multiply(&x);
    if y.square() == x.pow_vartime(&[3u64]).add(&a_x).add(&P256_B) {
        return Ok(true);
    }

    Ok(false)
}
