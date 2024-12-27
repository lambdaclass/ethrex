use bytes::Bytes;
use ethrex_core::{Address, H160, H256, U256};
use keccak_hash::keccak256;
use kzg_rs::{Bytes32, Bytes48, KzgSettings};
use lambdaworks_math::{
    cyclic_group::IsGroup,
    elliptic_curve::{
        short_weierstrass::{
            curves::bn_254::{
                curve::{BN254Curve, BN254FieldElement, BN254TwistCurveFieldElement},
                field_extension::Degree12ExtensionField,
                pairing::BN254AtePairing,
                twist::BN254TwistCurve,
            },
            point::ShortWeierstrassProjectivePoint,
        },
        traits::{IsEllipticCurve, IsPairing},
    },
    field::{element::FieldElement, extensions::quadratic::QuadraticExtensionFieldElement},
    traits::ByteConversion,
    unsigned_integer::element,
};
use libsecp256k1::{self, Message, RecoveryId, Signature};
use num_bigint::BigUint;
use sha3::Digest;

use crate::{
    call_frame::CallFrame,
    constants::VERSIONED_HASH_VERSION_KZG,
    errors::{InternalError, PrecompileError, VMError},
    gas_cost::{self, ECADD_COST, ECMUL_COST, ECRECOVER_COST, MODEXP_STATIC_COST},
};

pub const ECRECOVER_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01,
]);
pub const SHA2_256_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x02,
]);
pub const RIPEMD_160_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x03,
]);
pub const IDENTITY_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x04,
]);
pub const MODEXP_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x05,
]);
pub const ECADD_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x06,
]);
pub const ECMUL_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x07,
]);
pub const ECPAIRING_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x08,
]);
pub const BLAKE2F_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x09,
]);
pub const POINT_EVALUATION_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x0a,
]);

pub const PRECOMPILES: [H160; 10] = [
    ECRECOVER_ADDRESS,
    SHA2_256_ADDRESS,
    RIPEMD_160_ADDRESS,
    IDENTITY_ADDRESS,
    MODEXP_ADDRESS,
    ECADD_ADDRESS,
    ECMUL_ADDRESS,
    ECPAIRING_ADDRESS,
    BLAKE2F_ADDRESS,
    POINT_EVALUATION_ADDRESS,
];

pub fn is_precompile(callee_address: &Address) -> bool {
    PRECOMPILES.contains(callee_address)
}

pub fn execute_precompile(current_call_frame: &mut CallFrame) -> Result<Bytes, VMError> {
    let callee_address = current_call_frame.code_address;
    let calldata = current_call_frame.calldata.clone();
    let gas_for_call = current_call_frame
        .gas_limit
        .checked_sub(current_call_frame.gas_used)
        .ok_or(InternalError::ArithmeticOperationUnderflow)?;
    let consumed_gas = &mut current_call_frame.gas_used;

    let result = match callee_address {
        address if address == ECRECOVER_ADDRESS => {
            ecrecover(&calldata, gas_for_call, consumed_gas)?
        }
        address if address == IDENTITY_ADDRESS => identity(&calldata, gas_for_call, consumed_gas)?,
        address if address == SHA2_256_ADDRESS => sha2_256(&calldata, gas_for_call, consumed_gas)?,
        address if address == RIPEMD_160_ADDRESS => {
            ripemd_160(&calldata, gas_for_call, consumed_gas)?
        }
        address if address == MODEXP_ADDRESS => modexp(&calldata, gas_for_call, consumed_gas)?,
        address if address == ECADD_ADDRESS => ecadd(&calldata, gas_for_call, consumed_gas)?,
        address if address == ECMUL_ADDRESS => ecmul(&calldata, gas_for_call, consumed_gas)?,
        address if address == ECPAIRING_ADDRESS => {
            ecpairing(&calldata, gas_for_call, consumed_gas)?
        }
        address if address == BLAKE2F_ADDRESS => blake2f(&calldata, gas_for_call, consumed_gas)?,
        address if address == POINT_EVALUATION_ADDRESS => {
            point_evaluation(&calldata, gas_for_call, consumed_gas)?
        }
        _ => return Err(VMError::Internal(InternalError::InvalidPrecompileAddress)),
    };

    Ok(result)
}

/// Verifies if the gas cost is higher than the gas limit and consumes the gas cost if it is not
fn increase_precompile_consumed_gas(
    gas_for_call: u64,
    gas_cost: u64,
    consumed_gas: &mut u64,
) -> Result<(), VMError> {
    if gas_for_call < gas_cost {
        return Err(VMError::PrecompileError(PrecompileError::NotEnoughGas));
    }

    *consumed_gas = consumed_gas
        .checked_add(gas_cost)
        .ok_or(PrecompileError::GasConsumedOverflow)?;

    Ok(())
}

/// When slice length is less than 128, the rest is filled with zeros. If slice length is
/// more than 128 the excess bytes are discarded.
fn fill_with_zeros(calldata: &Bytes, target_len: usize) -> Result<Bytes, VMError> {
    let mut padded_calldata = calldata.to_vec();
    if padded_calldata.len() < target_len {
        let size_diff = target_len
            .checked_sub(padded_calldata.len())
            .ok_or(InternalError::ArithmeticOperationUnderflow)?;
        padded_calldata.extend(vec![0u8; size_diff]);
    }
    Ok(padded_calldata.into())
}

/// ECDSA (Elliptic curve digital signature algorithm) public key recovery function.
/// Given a hash, a Signature and a recovery Id, returns the public key recovered by secp256k1
pub fn ecrecover(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    let gas_cost = ECRECOVER_COST;

    increase_precompile_consumed_gas(gas_for_call, gas_cost, consumed_gas)?;

    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 128)?;

    // Parse the input elements, first as a slice of bytes and then as an specific type of the crate
    let hash = calldata.get(0..32).ok_or(InternalError::SlicingError)?;
    let Ok(message) = Message::parse_slice(hash) else {
        return Ok(Bytes::new());
    };

    let v: U256 = calldata
        .get(32..64)
        .ok_or(InternalError::SlicingError)?
        .into();

    // The Recovery identifier is expected to be 27 or 28, any other value is invalid
    if !(v == U256::from(27) || v == U256::from(28)) {
        return Ok(Bytes::new());
    }

    let v = u8::try_from(v).map_err(|_| InternalError::ConversionError)?;
    let Ok(recovery_id) = RecoveryId::parse_rpc(v) else {
        return Ok(Bytes::new());
    };

    // signature is made up of the parameters r and s
    let sig = calldata.get(64..128).ok_or(InternalError::SlicingError)?;
    let Ok(signature) = Signature::parse_standard_slice(sig) else {
        return Ok(Bytes::new());
    };

    // Recover the address using secp256k1
    let Ok(public_key) = libsecp256k1::recover(&message, &signature, &recovery_id) else {
        return Ok(Bytes::new());
    };

    let mut public_key = public_key.serialize();

    // We need to take the 64 bytes from the public key (discarding the first pos of the slice)
    keccak256(&mut public_key[1..65]);

    // The output is 32 bytes: the initial 12 bytes with 0s, and the remaining 20 with the recovered address
    let mut output = vec![0u8; 12];
    output.extend_from_slice(public_key.get(13..33).ok_or(InternalError::SlicingError)?);

    Ok(Bytes::from(output.to_vec()))
}

pub fn identity(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::identity(calldata.len())?;

    increase_precompile_consumed_gas(gas_for_call, gas_cost, consumed_gas)?;

    Ok(calldata.clone())
}

/// Returns the calldata hashed by sha2-256 algorithm
pub fn sha2_256(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::sha2_256(calldata.len())?;

    increase_precompile_consumed_gas(gas_for_call, gas_cost, consumed_gas)?;

    let result = sha2::Sha256::digest(calldata).to_vec();

    Ok(Bytes::from(result))
}

/// Returns the calldata hashed by ripemd-160 algorithm, padded by zeros at left
pub fn ripemd_160(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::ripemd_160(calldata.len())?;

    increase_precompile_consumed_gas(gas_for_call, gas_cost, consumed_gas)?;

    let mut hasher = ripemd::Ripemd160::new();
    hasher.update(calldata);
    let result = hasher.finalize();

    let mut output = vec![0; 12];
    output.extend_from_slice(&result);

    Ok(Bytes::from(output))
}

pub fn modexp(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 96)?;

    let b_size: U256 = calldata
        .get(0..32)
        .ok_or(PrecompileError::ParsingInputError)?
        .into();

    let e_size: U256 = calldata
        .get(32..64)
        .ok_or(PrecompileError::ParsingInputError)?
        .into();

    let m_size: U256 = calldata
        .get(64..96)
        .ok_or(PrecompileError::ParsingInputError)?
        .into();

    if b_size == U256::zero() && m_size == U256::zero() {
        increase_precompile_consumed_gas(gas_for_call, MODEXP_STATIC_COST, consumed_gas)?;
        return Ok(Bytes::new());
    }

    // Because on some cases conversions to usize exploded before the check of the zero value could be done
    let b_size = usize::try_from(b_size).map_err(|_| PrecompileError::ParsingInputError)?;
    let e_size = usize::try_from(e_size).map_err(|_| PrecompileError::ParsingInputError)?;
    let m_size = usize::try_from(m_size).map_err(|_| PrecompileError::ParsingInputError)?;

    let base_limit = b_size
        .checked_add(96)
        .ok_or(InternalError::ArithmeticOperationOverflow)?;

    let exponent_limit = e_size
        .checked_add(base_limit)
        .ok_or(InternalError::ArithmeticOperationOverflow)?;

    let modulus_limit = m_size
        .checked_add(exponent_limit)
        .ok_or(InternalError::ArithmeticOperationOverflow)?;
    // The reason I use unwrap_or_default is to cover the case where calldata does not reach the required
    // length, so then we should fill the rest with zeros. The same is done in modulus parsing
    let b = get_slice_or_default(&calldata, 96, base_limit, b_size)?;
    let base = BigUint::from_bytes_be(&b);

    let e = get_slice_or_default(&calldata, base_limit, exponent_limit, e_size)?;
    let exponent = BigUint::from_bytes_be(&e);

    let m = get_slice_or_default(&calldata, exponent_limit, modulus_limit, m_size)?;
    let modulus = BigUint::from_bytes_be(&m);

    // first 32 bytes of exponent or exponent if e_size < 32
    let bytes_to_take = 32.min(e_size);
    // Use of unwrap_or_default because if e == 0 get_slice_or_default returns an empty vec
    let exp_first_32 = BigUint::from_bytes_be(e.get(0..bytes_to_take).unwrap_or_default());

    let gas_cost = gas_cost::modexp(&exp_first_32, b_size, e_size, m_size)?;
    increase_precompile_consumed_gas(gas_for_call, gas_cost, consumed_gas)?;

    let result = mod_exp(base, exponent, modulus);

    let res_bytes = result.to_bytes_be();
    let res_bytes = increase_left_pad(&Bytes::from(res_bytes), m_size)?;

    Ok(res_bytes.slice(..m_size))
}

fn get_slice_or_default(
    calldata: &Bytes,
    lower_limit: usize,
    upper_limit: usize,
    size_to_expand: usize,
) -> Result<Vec<u8>, VMError> {
    let upper_limit = calldata.len().min(upper_limit);
    if let Some(data) = calldata.get(lower_limit..upper_limit) {
        if !data.is_empty() {
            let mut extended = vec![0u8; size_to_expand];
            for (dest, data) in extended.iter_mut().zip(data.iter()) {
                *dest = *data;
            }
            return Ok(extended);
        }
    }
    Ok(Default::default())
}

/// I allow this clippy alert because in the code modulus could never be
///  zero because that case is covered in the if above that line
#[allow(clippy::arithmetic_side_effects)]
fn mod_exp(base: BigUint, exponent: BigUint, modulus: BigUint) -> BigUint {
    if modulus == BigUint::ZERO {
        BigUint::ZERO
    } else if exponent == BigUint::ZERO {
        BigUint::from(1_u8) % modulus
    } else {
        base.modpow(&exponent, &modulus)
    }
}

pub fn increase_left_pad(result: &Bytes, m_size: usize) -> Result<Bytes, VMError> {
    let mut padded_result = vec![0u8; m_size];
    if result.len() < m_size {
        let size_diff = m_size
            .checked_sub(result.len())
            .ok_or(InternalError::ArithmeticOperationUnderflow)?;
        padded_result
            .get_mut(size_diff..)
            .ok_or(InternalError::SlicingError)?
            .copy_from_slice(result);

        Ok(padded_result.into())
    } else {
        Ok(result.clone())
    }
}

pub fn ecadd(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 128)?;
    increase_precompile_consumed_gas(gas_for_call, ECADD_COST, consumed_gas)?;
    let first_point_x = calldata
        .get(0..32)
        .ok_or(PrecompileError::ParsingInputError)?;

    let first_point_y = calldata
        .get(32..64)
        .ok_or(PrecompileError::ParsingInputError)?;

    let second_point_x = calldata
        .get(64..96)
        .ok_or(PrecompileError::ParsingInputError)?;

    let second_point_y = calldata
        .get(96..128)
        .ok_or(PrecompileError::ParsingInputError)?;

    // If points are zero the precompile should not fail, but the conversion in
    // BN254Curve::create_point_from_affine will, so we verify it before the conversion
    let first_point_is_zero = U256::from_big_endian(first_point_x).is_zero()
        && U256::from_big_endian(first_point_y).is_zero();
    let second_point_is_zero = U256::from_big_endian(second_point_x).is_zero()
        && U256::from_big_endian(second_point_y).is_zero();

    let first_point_x = BN254FieldElement::from_bytes_be(first_point_x)
        .map_err(|_| PrecompileError::ParsingInputError)?;
    let first_point_y = BN254FieldElement::from_bytes_be(first_point_y)
        .map_err(|_| PrecompileError::ParsingInputError)?;
    let second_point_x = BN254FieldElement::from_bytes_be(second_point_x)
        .map_err(|_| PrecompileError::ParsingInputError)?;
    let second_point_y = BN254FieldElement::from_bytes_be(second_point_y)
        .map_err(|_| PrecompileError::ParsingInputError)?;

    if first_point_is_zero && second_point_is_zero {
        // If both points are zero, return is zero
        Ok(Bytes::from([0u8; 64].to_vec()))
    } else if first_point_is_zero {
        // If first point is zero, return is second point
        let second_point = BN254Curve::create_point_from_affine(second_point_x, second_point_y)
            .map_err(|_| PrecompileError::ParsingInputError)?;
        let res = [
            second_point.x().to_bytes_be(),
            second_point.y().to_bytes_be(),
        ]
        .concat();
        Ok(Bytes::from(res))
    } else if second_point_is_zero {
        // If second point is zero, return is first point
        let first_point = BN254Curve::create_point_from_affine(first_point_x, first_point_y)
            .map_err(|_| PrecompileError::ParsingInputError)?;
        let res = [first_point.x().to_bytes_be(), first_point.y().to_bytes_be()].concat();
        Ok(Bytes::from(res))
    } else {
        // If none of the points is zero, return is the sum of both in the EC
        let first_point = BN254Curve::create_point_from_affine(first_point_x, first_point_y)
            .map_err(|_| PrecompileError::ParsingInputError)?;
        let second_point = BN254Curve::create_point_from_affine(second_point_x, second_point_y)
            .map_err(|_| PrecompileError::ParsingInputError)?;
        let sum = first_point.operate_with(&second_point).to_affine();

        if U256::from_big_endian(&sum.x().to_bytes_be()) == U256::zero()
            || U256::from_big_endian(&sum.y().to_bytes_be()) == U256::zero()
        {
            Ok(Bytes::from([0u8; 64].to_vec()))
        } else {
            let res = [sum.x().to_bytes_be(), sum.y().to_bytes_be()].concat();
            Ok(Bytes::from(res))
        }
    }
}

pub fn ecmul(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 96)?;

    increase_precompile_consumed_gas(gas_for_call, ECMUL_COST, consumed_gas)?;

    let point_x = calldata
        .get(0..32)
        .ok_or(PrecompileError::ParsingInputError)?;

    let point_y = calldata
        .get(32..64)
        .ok_or(PrecompileError::ParsingInputError)?;

    let scalar = calldata
        .get(64..96)
        .ok_or(PrecompileError::ParsingInputError)?;
    let scalar =
        element::U256::from_bytes_be(scalar).map_err(|_| PrecompileError::ParsingInputError)?;

    // If point is zero the precompile should not fail, but the conversion in
    // BN254Curve::create_point_from_affine will, so we verify it before the conversion
    let point_is_zero =
        U256::from_big_endian(point_x).is_zero() && U256::from_big_endian(point_y).is_zero();
    if point_is_zero {
        return Ok(Bytes::from([0u8; 64].to_vec()));
    }

    let point_x = BN254FieldElement::from_bytes_be(point_x)
        .map_err(|_| PrecompileError::ParsingInputError)?;
    let point_y = BN254FieldElement::from_bytes_be(point_y)
        .map_err(|_| PrecompileError::ParsingInputError)?;

    let point = BN254Curve::create_point_from_affine(point_x, point_y)
        .map_err(|_| PrecompileError::ParsingInputError)?;

    let zero_u256 = element::U256::from(0_u16);
    if scalar.eq(&zero_u256) {
        Ok(Bytes::from([0u8; 64].to_vec()))
    } else {
        let mul = point.operate_with_self(scalar).to_affine();
        if U256::from_big_endian(&mul.x().to_bytes_be()) == U256::zero()
            || U256::from_big_endian(&mul.y().to_bytes_be()) == U256::zero()
        {
            Ok(Bytes::from([0u8; 64].to_vec()))
        } else {
            let res = [mul.x().to_bytes_be(), mul.y().to_bytes_be()].concat();
            Ok(Bytes::from(res))
        }
    }
}

pub fn ecpairing(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    // The input must always be a multiple of 192 (6 32-byte values)
    if calldata.len() % 192 != 0 {
        return Err(VMError::PrecompileError(PrecompileError::ParsingInputError));
    }

    let inputs_amount = calldata.len() / 192;

    // Consume gas
    let gas_cost = gas_cost::ecpairing(inputs_amount)?;
    increase_precompile_consumed_gas(gas_for_call, gas_cost, consumed_gas)?;

    let mut mul: FieldElement<Degree12ExtensionField> = QuadraticExtensionFieldElement::one();
    for input_index in 0..inputs_amount {
        // Define the input indexes and slice calldata to get the input data
        let input_start = input_index
            .checked_mul(192)
            .ok_or(InternalError::ArithmeticOperationOverflow)?;
        let input_end = input_start
            .checked_add(192)
            .ok_or(InternalError::ArithmeticOperationOverflow)?;

        let input_data = calldata
            .get(input_start..input_end)
            .ok_or(InternalError::SlicingError)?;

        let first_point_x = input_data.get(..32).ok_or(InternalError::SlicingError)?;
        let first_point_y = input_data.get(32..64).ok_or(InternalError::SlicingError)?;

        // Infinite is defined by (0,0). Any other zero-combination is invalid
        if (U256::from_big_endian(first_point_x) == U256::zero())
            ^ (U256::from_big_endian(first_point_y) == U256::zero())
        {
            return Err(VMError::PrecompileError(PrecompileError::DefaultError));
        }

        let first_point_y = BN254FieldElement::from_bytes_be(first_point_y)
            .map_err(|_| PrecompileError::DefaultError)?;
        let first_point_x = BN254FieldElement::from_bytes_be(first_point_x)
            .map_err(|_| PrecompileError::DefaultError)?;

        let second_point_x_first_part =
            input_data.get(96..128).ok_or(InternalError::SlicingError)?;
        let second_point_x_second_part =
            input_data.get(64..96).ok_or(InternalError::SlicingError)?;

        // Infinite is defined by (0,0). Any other zero-combination is invalid
        if (U256::from_big_endian(second_point_x_first_part) == U256::zero())
            ^ (U256::from_big_endian(second_point_x_second_part) == U256::zero())
        {
            return Err(VMError::PrecompileError(PrecompileError::DefaultError));
        }

        let second_point_y_first_part = input_data
            .get(160..192)
            .ok_or(InternalError::SlicingError)?;
        let second_point_y_second_part = input_data
            .get(128..160)
            .ok_or(InternalError::SlicingError)?;

        // Infinite is defined by (0,0). Any other zero-combination is invalid
        if (U256::from_big_endian(second_point_y_first_part) == U256::zero())
            ^ (U256::from_big_endian(second_point_y_second_part) == U256::zero())
        {
            return Err(VMError::PrecompileError(PrecompileError::DefaultError));
        }

        let alt_bn128_prime = U256::from_str_radix(
            "30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd47",
            16,
        )
        .map_err(|_| InternalError::ConversionError)?;

        // Check if the second point belongs to the curve (this happens if it's lower than the prime)
        if U256::from_big_endian(second_point_x_first_part) >= alt_bn128_prime
            || U256::from_big_endian(second_point_x_second_part) >= alt_bn128_prime
            || U256::from_big_endian(second_point_y_first_part) >= alt_bn128_prime
            || U256::from_big_endian(second_point_y_second_part) >= alt_bn128_prime
        {
            return Err(VMError::PrecompileError(PrecompileError::DefaultError));
        }

        let second_point_x_bytes = [second_point_x_first_part, second_point_x_second_part].concat();
        let second_point_y_bytes = [second_point_y_first_part, second_point_y_second_part].concat();

        let second_point_x: FieldElement<lambdaworks_math::elliptic_curve::short_weierstrass::curves::bn_254::field_extension::Degree2ExtensionField> = BN254TwistCurveFieldElement::from_bytes_be(&second_point_x_bytes)
            .map_err(|_| PrecompileError::DefaultError)?;
        let second_point_y = BN254TwistCurveFieldElement::from_bytes_be(&second_point_y_bytes)
            .map_err(|_| PrecompileError::DefaultError)?;

        let zero_element = BN254FieldElement::from(0);
        let twcurve_zero_element = BN254TwistCurveFieldElement::from(0);
        let first_point_is_infinity =
            first_point_x.eq(&zero_element) && first_point_y.eq(&zero_element);
        let second_point_is_infinity =
            second_point_x.eq(&twcurve_zero_element) && second_point_y.eq(&twcurve_zero_element);

        match (first_point_is_infinity, second_point_is_infinity) {
            (true, true) => {
                // If both points are infinity, then continue to the next input
                continue;
            }
            (true, false) => {
                // If the first point is infinity, then do the checks for the second
                if let Ok(p2) = BN254TwistCurve::create_point_from_affine(
                    second_point_x.clone(),
                    second_point_y.clone(),
                ) {
                    if !p2.is_in_subgroup() {
                        return Err(VMError::PrecompileError(PrecompileError::DefaultError));
                    } else {
                        continue;
                    }
                } else {
                    return Err(VMError::PrecompileError(PrecompileError::DefaultError));
                }
            }
            (false, true) => {
                // If the second point is infinity, then do the checks for the first
                if BN254Curve::create_point_from_affine(
                    first_point_x.clone(),
                    first_point_y.clone(),
                )
                .is_err()
                {
                    return Err(VMError::PrecompileError(PrecompileError::DefaultError));
                }
                continue;
            }
            (false, false) => {
                // Define the pairing points
                let first_point =
                    BN254Curve::create_point_from_affine(first_point_x, first_point_y)
                        .map_err(|_| PrecompileError::DefaultError)?;

                let second_point =
                    BN254TwistCurve::create_point_from_affine(second_point_x, second_point_y)
                        .map_err(|_| PrecompileError::DefaultError)?;
                if !second_point.is_in_subgroup() {
                    return Err(VMError::PrecompileError(PrecompileError::DefaultError));
                }

                // Get the result of the pairing and affect the mul value with it
                update_pairing_result(&mut mul, first_point, second_point)?;
            }
        }
    }

    // Generate the result from the variable mul
    let success = mul.eq(&QuadraticExtensionFieldElement::one());
    let mut result = [0; 32];
    result[31] = u8::from(success);
    Ok(Bytes::from(result.to_vec()))
}

/// I allow this clippy alert because lib handles mul for the type and will not panic in case of overflow
#[allow(clippy::arithmetic_side_effects)]
fn update_pairing_result(
    mul: &mut FieldElement<Degree12ExtensionField>,
    first_point: ShortWeierstrassProjectivePoint<BN254Curve>,
    second_point: ShortWeierstrassProjectivePoint<BN254TwistCurve>,
) -> Result<(), VMError> {
    let pairing_result = BN254AtePairing::compute_batch(&[(&first_point, &second_point)])
        .map_err(|_| PrecompileError::DefaultError)?;

    *mul *= pairing_result;

    Ok(())
}

fn blake2f(
    _calldata: &Bytes,
    _gas_for_call: u64,
    _consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    Ok(Bytes::new())
}

// Taken from the same name function from crates/common/types/blobs_bundle.rs
fn kzg_commitment_to_versioned_hash(data: &[u8; 48]) -> H256 {
    use k256::sha2::Digest;
    let mut versioned_hash: [u8; 32] = k256::sha2::Sha256::digest(data).into();
    versioned_hash[0] = VERSIONED_HASH_VERSION_KZG;
    versioned_hash.into()
}

fn verify_kzg_proof(
    commitment_bytes: &[u8; 48],
    x: &[u8; 32],
    y: &[u8; 32],
    proof_bytes: &[u8; 48],
) -> Result<bool, VMError> {
    let commitment_bytes =
        Bytes48::from_slice(commitment_bytes).map_err(|_| PrecompileError::DefaultError)?;
    let z_bytes = Bytes32::from_slice(x).map_err(|_| PrecompileError::DefaultError)?;
    let y_bytes = Bytes32::from_slice(y).map_err(|_| PrecompileError::DefaultError)?;
    let proof_bytes =
        Bytes48::from_slice(proof_bytes).map_err(|_| PrecompileError::DefaultError)?;

    let settings =
        KzgSettings::load_trusted_setup_file().map_err(|_| PrecompileError::DefaultError)?;

    kzg_rs::kzg_proof::KzgProof::verify_kzg_proof(
        &commitment_bytes,
        &z_bytes,
        &y_bytes,
        &proof_bytes,
        &settings,
    )
    .map_err(|_| VMError::PrecompileError(PrecompileError::DefaultError))
}

fn point_evaluation(
    calldata: &Bytes,
    gas_for_call: u64,
    consumed_gas: &mut u64,
) -> Result<Bytes, VMError> {
    if calldata.len() != 192 {
        return Err(VMError::PrecompileError(PrecompileError::ParsingInputError));
    }

    // Consume gas
    let gas_cost = 50000;
    increase_precompile_consumed_gas(gas_for_call, gas_cost, consumed_gas)?;

    // Parse inputs
    let hash = calldata
        .get(..32)
        .ok_or(PrecompileError::ParsingInputError)?;
    let mut hash_bytes = [0; 32];
    hash_bytes.copy_from_slice(hash);
    let versioned_hash = H256::from(hash_bytes);

    let x = calldata
        .get(32..64)
        .ok_or(PrecompileError::ParsingInputError)?;
    let mut x_bytes = [0; 32];
    x_bytes.copy_from_slice(x);

    let y = calldata
        .get(64..96)
        .ok_or(PrecompileError::ParsingInputError)?;
    let mut y_bytes = [0; 32];
    y_bytes.copy_from_slice(y);

    let commitment = calldata
        .get(96..144)
        .ok_or(PrecompileError::ParsingInputError)?;
    let mut commitment_bytes = [0; 48];
    commitment_bytes.copy_from_slice(commitment);

    let proof = calldata
        .get(144..192)
        .ok_or(PrecompileError::ParsingInputError)?;
    let mut proof_bytes = [0; 48];
    proof_bytes.copy_from_slice(proof);

    // Perform the evaluation

    // This checks if the commitment is equal to the versioned hash
    if kzg_commitment_to_versioned_hash(&commitment_bytes) != versioned_hash {
        return Err(VMError::PrecompileError(PrecompileError::ParsingInputError));
    }

    // This verifies the proof from a point (x, y) and a commitment
    if !verify_kzg_proof(&commitment_bytes, &x_bytes, &y_bytes, &proof_bytes).unwrap_or(false) {
        return Err(VMError::PrecompileError(PrecompileError::ParsingInputError));
    }

    // Create the output (it's always the same)
    let mut output: Vec<u8> = Vec::new();

    let field_elem_u256 = U256::from_dec_str("4096").unwrap_or_default();
    let mut field_elems_per_blob = [0_u8; 32];
    field_elem_u256.to_big_endian(&mut field_elems_per_blob);

    let number = U256::from_dec_str(
        "52435875175126190479447740508185965837690552500527637822603658699938581184513",
    )
    .unwrap_or_default();
    let mut bls_modulus = [0u8; 32];
    number.to_big_endian(&mut bls_modulus);

    output.extend_from_slice(&field_elems_per_blob);
    output.extend_from_slice(&bls_modulus);

    Ok(Bytes::from(output))
}
