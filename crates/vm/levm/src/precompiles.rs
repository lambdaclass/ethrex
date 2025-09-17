use ark_bn254::{Fr as FrArk, G1Affine as G1AffineArk};
use ark_ec::CurveGroup;
use ark_ff::{BigInteger, PrimeField as ArkPrimeField, Zero};
use bls12_381::{
    Fp, Fp2, G1Affine, G1Projective, G2Affine, G2Prepared, G2Projective, Gt, Scalar,
    hash_to_curve::MapToCurve, multi_miller_loop,
};
use bytes::{Buf, Bytes};
use ethrex_common::utils::u256_from_big_endian_const;
use ethrex_common::{
    Address, H256, U256, addresses::*, kzg::verify_kzg_proof, serde_utils::bool, types::Fork,
    utils::u256_from_big_endian,
};
use ethrex_crypto::blake2f::blake2b_f;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use k256::elliptic_curve::Field;
use keccak_hash::keccak256;
use lambdaworks_math::{
    elliptic_curve::{
        short_weierstrass::{
            curves::bn_254::{
                curve::{BN254Curve, BN254FieldElement, BN254TwistCurveFieldElement},
                field_extension::{
                    BN254_PRIME_FIELD_ORDER, BN254FieldModulus, Degree2ExtensionField,
                    Degree12ExtensionField,
                },
                pairing::BN254AtePairing,
                twist::BN254TwistCurve,
            },
            point::ShortWeierstrassProjectivePoint,
        },
        traits::{IsEllipticCurve, IsPairing},
    },
    field::{
        element::FieldElement, extensions::quadratic::QuadraticExtensionFieldElement,
        fields::montgomery_backed_prime_fields::MontgomeryBackendPrimeField,
    },
    traits::ByteConversion,
    unsigned_integer::element::UnsignedInteger,
};
use malachite::base::num::arithmetic::traits::ModPow as _;
use malachite::base::num::basic::traits::Zero as _;
use malachite::{Natural, base::num::conversion::traits::*};
use p256::{
    EncodedPoint, FieldElement as P256FieldElement,
    ecdsa::{Signature as P256Signature, signature::hazmat::PrehashVerifier},
    elliptic_curve::bigint::U256 as P256Uint,
};
use sha3::Digest;
use std::borrow::Cow;
use std::ops::Mul;

use crate::constants::{P256_A, P256_B, P256_N};
use crate::gas_cost::{MODEXP_STATIC_COST, P256_VERIFY_COST};
use crate::vm::VMType;
use crate::{
    constants::{P256_P, VERSIONED_HASH_VERSION_KZG},
    errors::{ExceptionalHalt, InternalError, PrecompileError, VMError},
    gas_cost::{
        self, BLAKE2F_ROUND_COST, BLS12_381_G1_K_DISCOUNT, BLS12_381_G1ADD_COST,
        BLS12_381_G2_K_DISCOUNT, BLS12_381_G2ADD_COST, BLS12_381_MAP_FP_TO_G1_COST,
        BLS12_381_MAP_FP2_TO_G2_COST, ECADD_COST, ECMUL_COST, ECRECOVER_COST, G1_MUL_COST,
        G2_MUL_COST, POINT_EVALUATION_COST,
    },
};

pub const BLAKE2F_ELEMENT_SIZE: usize = 8;

pub const SIZE_PRECOMPILES_PRE_CANCUN: u64 = 9;
pub const SIZE_PRECOMPILES_CANCUN: u64 = 10;
pub const SIZE_PRECOMPILES_PRAGUE: u64 = 17;

pub const BLS12_381_G1_MSM_PAIR_LENGTH: usize = 160;
pub const BLS12_381_G2_MSM_PAIR_LENGTH: usize = 288;
pub const BLS12_381_PAIRING_CHECK_PAIR_LENGTH: usize = 384;

const BLS12_381_G1ADD_VALID_INPUT_LENGTH: usize = 256;
const BLS12_381_G2ADD_VALID_INPUT_LENGTH: usize = 512;

const BLS12_381_FP2_VALID_INPUT_LENGTH: usize = 128;
const BLS12_381_FP_VALID_INPUT_LENGTH: usize = 64;

pub const FIELD_ELEMENT_WITHOUT_PADDING_LENGTH: usize = 48;
pub const PADDED_FIELD_ELEMENT_SIZE_IN_BYTES: usize = 64;

const FP2_ZERO_MAPPED_TO_G2: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 131, 32, 137, 110, 201, 238, 249, 213, 230,
    25, 132, 141, 194, 156, 226, 102, 244, 19, 208, 45, 211, 29, 155, 157, 68, 236, 12, 121, 205,
    97, 241, 139, 7, 93, 219, 166, 215, 189, 32, 183, 255, 39, 164, 179, 36, 191, 206, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 103, 209, 33, 24, 181, 163, 91, 176, 45, 46, 134, 179,
    235, 250, 126, 35, 65, 13, 185, 61, 227, 159, 176, 109, 112, 37, 250, 149, 233, 111, 250, 66,
    138, 122, 39, 195, 174, 77, 212, 180, 11, 210, 81, 172, 101, 136, 146, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 2, 96, 224, 54, 68, 209, 162, 195, 33, 37, 107, 50, 70, 186, 210, 184,
    149, 202, 209, 56, 144, 203, 230, 248, 93, 245, 81, 6, 160, 211, 52, 96, 79, 177, 67, 199, 160,
    66, 216, 120, 0, 98, 113, 134, 91, 195, 89, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    4, 198, 151, 119, 164, 63, 11, 218, 7, 103, 157, 88, 5, 230, 63, 24, 207, 78, 14, 124, 97, 18,
    172, 127, 112, 38, 109, 25, 155, 79, 118, 174, 39, 198, 38, 154, 60, 238, 189, 174, 48, 128,
    110, 154, 118, 170, 223, 92,
];
pub const G1_POINT_AT_INFINITY: [u8; 128] = [0_u8; 128];
pub const G2_POINT_AT_INFINITY: [u8; 256] = [0_u8; 256];

pub fn is_precompile(address: &Address, fork: Fork, vm_type: VMType) -> bool {
    // Cancun specs is the only one that allows point evaluation precompile
    if *address == POINT_EVALUATION_ADDRESS && fork < Fork::Cancun {
        return false;
    }
    // Prague or newers forks should only use these precompiles
    // https://eips.ethereum.org/EIPS/eip-2537
    if PRECOMPILES_POST_CANCUN.contains(address) && fork < Fork::Prague {
        return false;
    }

    // P256 verify Precompile only existed on L2 before Osaka
    if fork < Fork::Osaka && matches!(vm_type, VMType::L1) && address == &P256_VERIFICATION_ADDRESS
    {
        return false;
    }

    PRECOMPILES.contains(address)
        || PRECOMPILES_POST_CANCUN.contains(address)
        || address == &P256_VERIFICATION_ADDRESS
}

#[expect(clippy::as_conversions, clippy::indexing_slicing)]
pub fn execute_precompile(
    address: Address,
    calldata: &Bytes,
    gas_remaining: &mut u64,
    fork: Fork,
) -> Result<Bytes, VMError> {
    type PrecompileFn = fn(&Bytes, &mut u64, Fork) -> Result<Bytes, VMError>;

    const PRECOMPILES: [Option<PrecompileFn>; 512] = const {
        let mut precompiles = [const { None }; 512];
        precompiles[ECRECOVER_ADDRESS.0[19] as usize] = Some(ecrecover as PrecompileFn);
        precompiles[IDENTITY_ADDRESS.0[19] as usize] = Some(identity as PrecompileFn);
        precompiles[SHA2_256_ADDRESS.0[19] as usize] = Some(sha2_256 as PrecompileFn);
        precompiles[RIPEMD_160_ADDRESS.0[19] as usize] = Some(ripemd_160 as PrecompileFn);
        precompiles[MODEXP_ADDRESS.0[19] as usize] = Some(modexp as PrecompileFn);
        precompiles[ECADD_ADDRESS.0[19] as usize] = Some(ecadd as PrecompileFn);
        precompiles[ECMUL_ADDRESS.0[19] as usize] = Some(ecmul as PrecompileFn);
        precompiles[ECPAIRING_ADDRESS.0[19] as usize] = Some(ecpairing as PrecompileFn);
        precompiles[BLAKE2F_ADDRESS.0[19] as usize] = Some(blake2f as PrecompileFn);
        precompiles[POINT_EVALUATION_ADDRESS.0[19] as usize] =
            Some(point_evaluation as PrecompileFn);
        precompiles[BLS12_G1ADD_ADDRESS.0[19] as usize] = Some(bls12_g1add as PrecompileFn);
        precompiles[BLS12_G1MSM_ADDRESS.0[19] as usize] = Some(bls12_g1msm as PrecompileFn);
        precompiles[BLS12_G2ADD_ADDRESS.0[19] as usize] = Some(bls12_g2add as PrecompileFn);
        precompiles[BLS12_G2MSM_ADDRESS.0[19] as usize] = Some(bls12_g2msm as PrecompileFn);
        precompiles[BLS12_PAIRING_CHECK_ADDRESS.0[19] as usize] =
            Some(bls12_pairing_check as PrecompileFn);
        precompiles[BLS12_MAP_FP_TO_G1_ADDRESS.0[19] as usize] =
            Some(bls12_map_fp_to_g1 as PrecompileFn);
        precompiles[BLS12_MAP_FP2_TO_G2_ADDRESS.0[19] as usize] =
            Some(bls12_map_fp2_tp_g2 as PrecompileFn);
        precompiles[u16::from_be_bytes([
            P256_VERIFICATION_ADDRESS.0[18],
            P256_VERIFICATION_ADDRESS.0[19],
        ]) as usize] = Some(p_256_verify as PrecompileFn);
        precompiles
    };

    if address[0..17] != [0u8; 17] {
        return Err(VMError::Internal(InternalError::InvalidPrecompileAddress));
    }
    let index = u16::from_be_bytes([address[18], address[19]]) as usize;

    let precompile = PRECOMPILES
        .get(index)
        .copied()
        .flatten()
        .ok_or(VMError::Internal(InternalError::InvalidPrecompileAddress))?;

    precompile(calldata, gas_remaining, fork)
}

/// Consumes gas and if it's higher than the gas limit returns an error.
pub(crate) fn increase_precompile_consumed_gas(
    gas_cost: u64,
    gas_remaining: &mut u64,
) -> Result<(), VMError> {
    *gas_remaining = gas_remaining
        .checked_sub(gas_cost)
        .ok_or(PrecompileError::NotEnoughGas)?;
    Ok(())
}

/// When slice length is less than `target_len`, the rest is filled with zeros. If slice length is
/// more than `target_len`, the excess bytes are kept.
#[inline(always)]
pub(crate) fn fill_with_zeros(calldata: &Bytes, target_len: usize) -> Bytes {
    if calldata.len() >= target_len {
        // this clone is cheap (Arc)
        return calldata.clone();
    }
    let mut padded_calldata = calldata.to_vec();
    padded_calldata.resize(target_len, 0);
    padded_calldata.into()
}

/// ## ECRECOVER precompile.
/// Elliptic curve digital signature algorithm (ECDSA) public key recovery function.
///
/// Input is 128 bytes (padded with zeros if shorter):
///   [0..32)  : keccak-256 hash (message digest)
///   [32..64) : v (27 or 28)
///   [64..128): r||s (64 bytes)
///
/// Returns the recovered address.
pub fn ecrecover(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    increase_precompile_consumed_gas(ECRECOVER_COST, gas_remaining)?;

    const INPUT_LEN: usize = 128;
    const WORD: usize = 32;

    let input = fill_with_zeros(calldata, INPUT_LEN);

    let (raw_hash, tail) = input.split_at(WORD);
    let (raw_v, raw_sig) = tail.split_at(WORD);

    // EVM expects v ∈ {27, 28}. Anything else is invalid → empty return.
    let v = match u8::try_from(u256_from_big_endian(raw_v)) {
        Ok(v @ (27 | 28)) => v,
        _ => return Ok(Bytes::new()),
    };

    #[allow(clippy::arithmetic_side_effects, reason = "v ∈ {27, 28}")]
    let mut recid_byte = v - 27;

    // Parse signature (r||s). If malformed → empty return.
    let mut sig = match Signature::from_slice(raw_sig) {
        Ok(s) => s,
        Err(_) => return Ok(Bytes::new()),
    };

    // k256 enforces canonical low-S for recovery.
    // If S is high, normalize s := n - s and flip the recovery parity bit.
    if let Some(low_s) = sig.normalize_s() {
        sig = low_s;
        recid_byte ^= 1;
    }

    // Recovery id from the adjusted byte.
    let recid = match RecoveryId::from_byte(recid_byte) {
        Some(id) => id,
        None => return Ok(Bytes::new()),
    };

    // Recover the verifying key from the prehash (32-byte digest).
    let vk = match VerifyingKey::recover_from_prehash(raw_hash, &sig, recid) {
        Ok(k) => k,
        Err(_) => return Ok(Bytes::new()),
    };

    // SEC1 uncompressed: 0x04 || X(32) || Y(32). We need X||Y (64 bytes).
    let uncompressed = vk.to_encoded_point(false);
    let mut uncompressed = uncompressed.to_bytes();
    #[allow(clippy::indexing_slicing)]
    let xy = &mut uncompressed[1..65];

    // keccak256(X||Y).
    keccak256(xy);

    // Address is the last 20 bytes of the hash.
    let mut out = [0u8; 32];
    #[allow(clippy::indexing_slicing)]
    out[12..32].copy_from_slice(&xy[12..32]);

    Ok(Bytes::copy_from_slice(&out))
}

/// Returns the calldata received
pub fn identity(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::identity(calldata.len())?;

    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    Ok(calldata.clone())
}

/// Returns the calldata hashed by sha2-256 algorithm
pub fn sha2_256(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::sha2_256(calldata.len())?;

    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let digest = sha2::Sha256::digest(calldata);
    let result = digest.as_slice();

    Ok(Bytes::copy_from_slice(result))
}

/// Returns the calldata hashed by ripemd-160 algorithm, padded by zeros at left
pub fn ripemd_160(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::ripemd_160(calldata.len())?;

    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let mut hasher = ripemd::Ripemd160::new();
    hasher.update(calldata);
    let result = hasher.finalize();

    let mut output = vec![0; 12];
    output.extend_from_slice(&result);

    Ok(Bytes::from(output))
}

/// Returns the result of the module-exponentiation operation
#[expect(clippy::indexing_slicing, reason = "bounds checked at start")]
pub fn modexp(calldata: &Bytes, gas_remaining: &mut u64, fork: Fork) -> Result<Bytes, VMError> {
    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 96);

    // Defer converting to a U256 after the zero check.
    if fork < Fork::Osaka {
        let base_size_bytes: [u8; 32] = calldata[0..32].try_into()?;
        let modulus_size_bytes: [u8; 32] = calldata[64..96].try_into()?;
        const ZERO_BYTES: [u8; 32] = [0u8; 32];

        if base_size_bytes == ZERO_BYTES && modulus_size_bytes == ZERO_BYTES {
            // On Berlin or newer there is a floor cost for the modexp precompile
            increase_precompile_consumed_gas(MODEXP_STATIC_COST, gas_remaining)?;
            return Ok(Bytes::new());
        }
    }

    // The try_into are infallible and the compiler optimizes them out, even without unsafe.
    // https://godbolt.org/z/h8rW8M3c4
    let base_size = u256_from_big_endian_const::<32>(calldata[0..32].try_into()?);
    let modulus_size = u256_from_big_endian_const::<32>(calldata[64..96].try_into()?);
    let exponent_size = u256_from_big_endian_const::<32>(calldata[32..64].try_into()?);

    if fork >= Fork::Osaka {
        if base_size > U256::from(1024) {
            return Err(PrecompileError::ModExpBaseTooLarge.into());
        }
        if exponent_size > U256::from(1024) {
            return Err(PrecompileError::ModExpExpTooLarge.into());
        }
        if modulus_size > U256::from(1024) {
            return Err(PrecompileError::ModExpModulusTooLarge.into());
        }
    }

    // Because on some cases conversions to usize exploded before the check of the zero value could be done
    let base_size = usize::try_from(base_size).map_err(|_| PrecompileError::ParsingInputError)?;
    let exponent_size =
        usize::try_from(exponent_size).map_err(|_| PrecompileError::ParsingInputError)?;
    let modulus_size =
        usize::try_from(modulus_size).map_err(|_| PrecompileError::ParsingInputError)?;

    let base_limit = base_size.checked_add(96).ok_or(InternalError::Overflow)?;

    let exponent_limit = exponent_size
        .checked_add(base_limit)
        .ok_or(InternalError::Overflow)?;

    let modulus_limit = modulus_size
        .checked_add(exponent_limit)
        .ok_or(InternalError::Overflow)?;

    let b = get_slice_or_default(&calldata, 96, base_limit, base_size);
    let e = get_slice_or_default(&calldata, base_limit, exponent_limit, exponent_size);
    let m = get_slice_or_default(&calldata, exponent_limit, modulus_limit, modulus_size);

    let base = Natural::from_power_of_2_digits_desc(8u64, b.iter().cloned())
        .ok_or(InternalError::TypeConversion)?;
    let exponent = Natural::from_power_of_2_digits_desc(8u64, e.iter().cloned())
        .ok_or(InternalError::TypeConversion)?;
    let modulus = Natural::from_power_of_2_digits_desc(8u64, m.iter().cloned())
        .ok_or(InternalError::TypeConversion)?;

    // First 32 bytes of exponent or exponent if e_size < 32
    let bytes_to_take = 32.min(exponent_size);
    // Use of unwrap_or_default because if e == 0 get_slice_or_default returns an empty vec
    let exp_first_32 = Natural::from_power_of_2_digits_desc(
        8u64,
        e.get(0..bytes_to_take).unwrap_or_default().iter().cloned(),
    )
    .ok_or(InternalError::TypeConversion)?;

    let gas_cost = gas_cost::modexp(&exp_first_32, base_size, exponent_size, modulus_size, fork)?;

    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    if base_size == 0 && modulus_size == 0 {
        return Ok(Bytes::new());
    }

    let result = mod_exp(base, exponent, modulus);

    let res_bytes: Vec<u8> = result.to_power_of_2_digits_desc(8);
    let res_bytes = increase_left_pad(&Bytes::from(res_bytes), modulus_size);

    Ok(res_bytes.slice(..modulus_size))
}

/// This function returns the slice between the lower and upper limit of the calldata (as a vector),
/// padding with zeros at the end if necessary.
///
/// Uses Cow so that the best case of no resizing doesn't require an allocation.
#[expect(clippy::indexing_slicing, reason = "bounds checked")]
fn get_slice_or_default<'c>(
    calldata: &'c Bytes,
    lower_limit: usize,
    upper_limit: usize,
    size_to_expand: usize,
) -> Cow<'c, [u8]> {
    let upper_limit = calldata.len().min(upper_limit);
    if let Some(data) = calldata.get(lower_limit..upper_limit) {
        if !data.is_empty() {
            if data.len() == size_to_expand {
                return data.into();
            }
            let mut extended = vec![0u8; size_to_expand];
            let copy_size = size_to_expand.min(data.len());
            extended[..copy_size].copy_from_slice(&data[..copy_size]);
            return extended.into();
        }
    }
    Vec::new().into()
}

#[allow(clippy::arithmetic_side_effects)]
#[inline(always)]
fn mod_exp(base: Natural, exponent: Natural, modulus: Natural) -> Natural {
    if modulus == Natural::ZERO {
        Natural::ZERO
    } else if exponent == Natural::ZERO {
        Natural::from(1_u8) % modulus
    } else {
        let base_mod = base % &modulus; // malachite requires base to be reduced to modulus first
        base_mod.mod_pow(&exponent, &modulus)
    }
}

/// If the result size is less than needed, pads left with zeros.
#[inline(always)]
pub fn increase_left_pad(result: &Bytes, m_size: usize) -> Bytes {
    #[expect(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        reason = "overflow checked with the if condition, bounds checked"
    )]
    if result.len() < m_size {
        let mut padded_result = vec![0u8; m_size];
        let size_diff = m_size - result.len();
        padded_result[size_diff..].copy_from_slice(result);

        padded_result.into()
    } else {
        // this clone is cheap (Arc)
        result.clone()
    }
}

/// Makes a point addition on the elliptic curve 'alt_bn128'
pub fn ecadd(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 128);

    increase_precompile_consumed_gas(ECADD_COST, gas_remaining)?;

    let first_point_x = calldata.get(0..32).ok_or(InternalError::Slicing)?;
    let first_point_y = calldata.get(32..64).ok_or(InternalError::Slicing)?;
    let second_point_x = calldata.get(64..96).ok_or(InternalError::Slicing)?;
    let second_point_y = calldata.get(96..128).ok_or(InternalError::Slicing)?;

    if u256_from_big_endian(first_point_x) >= ALT_BN128_PRIME
        || u256_from_big_endian(first_point_y) >= ALT_BN128_PRIME
        || u256_from_big_endian(second_point_x) >= ALT_BN128_PRIME
        || u256_from_big_endian(second_point_y) >= ALT_BN128_PRIME
    {
        return Err(PrecompileError::InvalidPoint.into());
    }

    let first_point_x = ark_bn254::Fq::from_be_bytes_mod_order(first_point_x);
    let first_point_y = ark_bn254::Fq::from_be_bytes_mod_order(first_point_y);
    let second_point_x = ark_bn254::Fq::from_be_bytes_mod_order(second_point_x);
    let second_point_y = ark_bn254::Fq::from_be_bytes_mod_order(second_point_y);

    let first_point_is_zero = first_point_x.is_zero() && first_point_y.is_zero();
    let second_point_is_zero = second_point_x.is_zero() && second_point_y.is_zero();

    let result: G1AffineArk = match (first_point_is_zero, second_point_is_zero) {
        (true, true) => {
            return Ok(Bytes::from([0u8; 64].to_vec()));
        }
        (false, true) => {
            let first_point = G1AffineArk::new_unchecked(first_point_x, first_point_y);
            if !first_point.is_on_curve() {
                return Err(PrecompileError::InvalidPoint.into());
            }
            first_point
        }
        (true, false) => {
            let second_point = G1AffineArk::new_unchecked(second_point_x, second_point_y);
            if !second_point.is_on_curve() {
                return Err(PrecompileError::InvalidPoint.into());
            }
            second_point
        }
        (false, false) => {
            let first_point = G1AffineArk::new_unchecked(first_point_x, first_point_y);
            if !first_point.is_on_curve() {
                return Err(PrecompileError::InvalidPoint.into());
            }
            let second_point = G1AffineArk::new_unchecked(second_point_x, second_point_y);
            if !second_point.is_on_curve() {
                return Err(PrecompileError::InvalidPoint.into());
            }
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "Valid operation between two elliptic curve points"
            )]
            let sum = first_point + second_point;
            sum.into_affine()
        }
    };

    let out = [
        result.x.into_bigint().to_bytes_be(),
        result.y.into_bigint().to_bytes_be(),
    ]
    .concat();

    Ok(Bytes::from(out))
}

/// Makes a scalar multiplication on the elliptic curve 'alt_bn128'
pub fn ecmul(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 96);

    increase_precompile_consumed_gas(ECMUL_COST, gas_remaining)?;

    let point_x = calldata.get(0..32).ok_or(InternalError::Slicing)?;
    let point_y = calldata.get(32..64).ok_or(InternalError::Slicing)?;
    let scalar = calldata.get(64..96).ok_or(InternalError::Slicing)?;

    if u256_from_big_endian(point_x) >= ALT_BN128_PRIME
        || u256_from_big_endian(point_y) >= ALT_BN128_PRIME
    {
        return Err(PrecompileError::InvalidPoint.into());
    }

    let x = ark_bn254::Fq::from_be_bytes_mod_order(point_x);
    let y = ark_bn254::Fq::from_be_bytes_mod_order(point_y);

    if x.is_zero() && y.is_zero() {
        return Ok(Bytes::from([0u8; 64].to_vec()));
    }

    let point = G1AffineArk::new_unchecked(x, y);
    if !point.is_on_curve() {
        return Err(PrecompileError::InvalidPoint.into());
    }

    let scalar = FrArk::from_be_bytes_mod_order(scalar);
    if scalar.is_zero() {
        return Ok(Bytes::from([0u8; 64].to_vec()));
    }

    let result = point.mul(scalar).into_affine();

    let out = [
        result.x.into_bigint().to_bytes_be(),
        result.y.into_bigint().to_bytes_be(),
    ]
    .concat();

    Ok(Bytes::from(out))
}

const ALT_BN128_PRIME: U256 = U256([
    0x3c208c16d87cfd47,
    0x97816a916871ca8d,
    0xb85045b68181585d,
    0x30644e72e131a029,
]);

type FirstPointCoordinates = (
    FieldElement<MontgomeryBackendPrimeField<BN254FieldModulus, 4>>,
    FieldElement<MontgomeryBackendPrimeField<BN254FieldModulus, 4>>,
);

/// Parses first point coordinates and makes verification of invalid infinite
#[inline]
fn parse_first_point_coordinates(input_data: &[u8; 192]) -> Result<FirstPointCoordinates, VMError> {
    let first_point_x = UnsignedInteger::from_bytes_be(&input_data[..32])
        .map_err(|_| InternalError::msg("Failed to create BN254 element from bytes"))?;
    let first_point_y = UnsignedInteger::from_bytes_be(&input_data[32..64])
        .map_err(|_| InternalError::msg("Failed to create BN254 element from bytes"))?;
    // Infinite is defined by (0,0). Any other zero-combination is invalid
    if (first_point_x == UnsignedInteger::default()) ^ (first_point_y == UnsignedInteger::default())
    {
        return Err(PrecompileError::InvalidPoint.into());
    }

    if first_point_x > BN254_PRIME_FIELD_ORDER || first_point_y > BN254_PRIME_FIELD_ORDER {
        return Err(PrecompileError::CoordinateExceedsFieldModulus.into());
    }

    let first_point_x = BN254FieldElement::from(&first_point_x);
    let first_point_y = BN254FieldElement::from(&first_point_y);

    Ok((first_point_x, first_point_y))
}

/// Parses second point coordinates and makes verification of invalid infinite and curve belonging.
///
/// Slice must have len of 192. This function is only called from ecpairing which ensures that.
fn parse_second_point_coordinates(
    input_data: &[u8; 192],
) -> Result<
    (
        FieldElement<Degree2ExtensionField>,
        FieldElement<Degree2ExtensionField>,
    ),
    VMError,
> {
    let second_point_x_first_part = &input_data[96..128];
    let second_point_x_second_part = &input_data[64..96];

    // Infinite is defined by (0,0). Any other zero-combination is invalid
    if (u256_from_big_endian(second_point_x_first_part) == U256::zero())
        ^ (u256_from_big_endian(second_point_x_second_part) == U256::zero())
    {
        return Err(PrecompileError::InvalidPoint.into());
    }

    let second_point_y_first_part = &input_data[160..192];
    let second_point_y_second_part = &input_data[128..160];

    // Infinite is defined by (0,0). Any other zero-combination is invalid
    if (u256_from_big_endian(second_point_y_first_part) == U256::zero())
        ^ (u256_from_big_endian(second_point_y_second_part) == U256::zero())
    {
        return Err(PrecompileError::InvalidPoint.into());
    }

    // Check if the second point belongs to the curve (this happens if it's lower than the prime)
    if u256_from_big_endian(second_point_x_first_part) >= ALT_BN128_PRIME
        || u256_from_big_endian(second_point_x_second_part) >= ALT_BN128_PRIME
        || u256_from_big_endian(second_point_y_first_part) >= ALT_BN128_PRIME
        || u256_from_big_endian(second_point_y_second_part) >= ALT_BN128_PRIME
    {
        return Err(PrecompileError::PointNotInTheCurve.into());
    }

    let second_point_x_bytes = [second_point_x_first_part, second_point_x_second_part].concat();
    let second_point_y_bytes = [second_point_y_first_part, second_point_y_second_part].concat();

    let second_point_x = BN254TwistCurveFieldElement::from_bytes_be(&second_point_x_bytes)
        .map_err(|_| InternalError::msg("Failed to create BN254 element from bytes"))?;
    let second_point_y = BN254TwistCurveFieldElement::from_bytes_be(&second_point_y_bytes)
        .map_err(|_| InternalError::msg("Failed to create BN254 element from bytes"))?;

    Ok((second_point_x, second_point_y))
}

/// Handles pairing given a certain elements, and depending on if elements represent infinity, then
/// verifies errors on the other point returning None or returns the pairing
#[inline(always)] // called only from one place, so inlining always wont increase code size.
#[expect(clippy::type_complexity)]
fn validate_pairing(
    first_point_x: FieldElement<MontgomeryBackendPrimeField<BN254FieldModulus, 4>>,
    first_point_y: FieldElement<MontgomeryBackendPrimeField<BN254FieldModulus, 4>>,
    second_point_x: FieldElement<Degree2ExtensionField>,
    second_point_y: FieldElement<Degree2ExtensionField>,
) -> Result<
    Option<(
        ShortWeierstrassProjectivePoint<BN254Curve>,
        ShortWeierstrassProjectivePoint<BN254TwistCurve>,
    )>,
    VMError,
> {
    let zero_element = BN254FieldElement::zero();
    let twcurve_zero_element = BN254TwistCurveFieldElement::zero();
    let first_point_is_infinity =
        first_point_x.eq(&zero_element) && first_point_y.eq(&zero_element);
    let second_point_is_infinity =
        second_point_x.eq(&twcurve_zero_element) && second_point_y.eq(&twcurve_zero_element);

    match (first_point_is_infinity, second_point_is_infinity) {
        (true, true) => {
            // If both points are infinity, then continue to the next input
            Ok(None)
        }
        (true, false) => {
            // If the first point is infinity, then do the checks for the second
            let p2 = BN254TwistCurve::create_point_from_affine(second_point_x, second_point_y)
                .map_err(|_| PrecompileError::InvalidPoint)?;

            if !p2.is_in_subgroup() {
                return Err(PrecompileError::PointNotInSubgroup.into());
            }
            Ok(None)
        }
        (false, true) => {
            // If the second point is infinity, then do the checks for the first
            BN254Curve::create_point_from_affine(first_point_x, first_point_y)
                .map_err(|_| PrecompileError::InvalidPoint)?;
            Ok(None)
        }
        (false, false) => {
            // Define the pairing points
            let first_point = BN254Curve::create_point_from_affine(first_point_x, first_point_y)
                .map_err(|_| PrecompileError::InvalidPoint)?;

            let second_point =
                BN254TwistCurve::create_point_from_affine(second_point_x, second_point_y)
                    .map_err(|_| PrecompileError::InvalidPoint)?;
            if !second_point.is_in_subgroup() {
                return Err(PrecompileError::PointNotInSubgroup.into());
            }
            Ok(Some((first_point, second_point)))
        }
    }
}

/// Performs a bilinear pairing on points on the elliptic curve 'alt_bn128', returns 1 on success and 0 on failure
pub fn ecpairing(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    // The input must always be a multiple of 192 (6 32-byte values)
    if calldata.len() % 192 != 0 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let inputs_amount = calldata.len() / 192;

    // Consume gas
    let gas_cost = gas_cost::ecpairing(inputs_amount)?;
    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let mut valid_pairs = Vec::new();
    let mut mul: FieldElement<Degree12ExtensionField> = QuadraticExtensionFieldElement::one();

    for input in calldata.chunks_exact(192) {
        #[expect(unsafe_code, reason = "chunks_exact ensures the conversion is valid")]
        let input: [u8; 192] = unsafe { input.try_into().unwrap_unchecked() };

        let (first_point_x, first_point_y) = parse_first_point_coordinates(&input)?;

        let (second_point_x, second_point_y) = parse_second_point_coordinates(&input)?;

        if let Some(pair) =
            validate_pairing(first_point_x, first_point_y, second_point_x, second_point_y)?
        {
            valid_pairs.push(pair);
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "library will not panic on mul overflow"
    )]
    if !valid_pairs.is_empty() {
        let batch: Vec<_> = valid_pairs.iter().map(|(p1, p2)| (p1, p2)).collect();
        let pairing_result = BN254AtePairing::compute_batch(&batch)
            .map_err(|_| PrecompileError::BN254AtePairingError)?;
        mul *= pairing_result;
    }

    // Generate the result from the variable mul
    let success = mul.eq(&QuadraticExtensionFieldElement::one());
    let mut result = [0; 32];
    result[31] = u8::from(success);
    Ok(Bytes::from(result.to_vec()))
}

/// Returns the result of Blake2 hashing algorithm given a certain parameters from the calldata.
pub fn blake2f(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    if calldata.len() != 213 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let mut calldata = calldata.slice(0..213);

    let rounds = calldata.get_u32();

    let gas_cost = u64::from(rounds) * BLAKE2F_ROUND_COST;
    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let mut h = [0; 8];

    h.copy_from_slice(&std::array::from_fn::<u64, 8, _>(|_| calldata.get_u64_le()));

    let mut m = [0; 16];

    m.copy_from_slice(&std::array::from_fn::<u64, 16, _>(|_| {
        calldata.get_u64_le()
    }));

    let mut t = [0; 2];
    t.copy_from_slice(&std::array::from_fn::<u64, 2, _>(|_| calldata.get_u64_le()));

    let f = calldata.get_u8();
    if f != 0 && f != 1 {
        return Err(PrecompileError::ParsingInputError.into());
    }
    let f = f == 1;

    #[expect(clippy::as_conversions)] // safe to convert a u32 to usize
    blake2b_f(rounds as usize, &mut h, &m, &t, f);

    Ok(Bytes::from_iter(
        h.into_iter().flat_map(|value| value.to_le_bytes()),
    ))
}

/// Converts the provided commitment to match the provided versioned_hash.
/// Taken from the same name function from crates/common/types/blobs_bundle.rs
fn kzg_commitment_to_versioned_hash(commitment_bytes: &[u8; 48]) -> H256 {
    use sha2::{Digest, Sha256};
    let mut versioned_hash: [u8; 32] = Sha256::digest(commitment_bytes).into();
    versioned_hash[0] = VERSIONED_HASH_VERSION_KZG;
    versioned_hash.into()
}

const POINT_EVALUATION_OUTPUT_BYTES: [u8; 64] = [
    // Big endian FIELD_ELEMENTS_PER_BLOB bytes
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00,
    // Big endian BLS_MODULUS bytes
    0x73, 0xED, 0xA7, 0x53, 0x29, 0x9D, 0x7D, 0x48, 0x33, 0x39, 0xD8, 0x08, 0x09, 0xA1, 0xD8, 0x05,
    0x53, 0xBD, 0xA4, 0x02, 0xFF, 0xFE, 0x5B, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x01,
];

/// Makes verifications on the received point, proof and commitment, if true returns a constant value
fn point_evaluation(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.len() != 192 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // Consume gas
    let gas_cost = POINT_EVALUATION_COST;
    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    // Parse inputs
    let versioned_hash: [u8; 32] = calldata
        .get(..32)
        .ok_or(InternalError::Slicing)?
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    let x: [u8; 32] = calldata
        .get(32..64)
        .ok_or(InternalError::Slicing)?
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    let y: [u8; 32] = calldata
        .get(64..96)
        .ok_or(InternalError::Slicing)?
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    let commitment: [u8; 48] = calldata
        .get(96..144)
        .ok_or(InternalError::Slicing)?
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    let proof: [u8; 48] = calldata
        .get(144..192)
        .ok_or(InternalError::Slicing)?
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    // Perform the evaluation

    // This checks if the commitment is equal to the versioned hash
    if kzg_commitment_to_versioned_hash(&commitment) != H256::from(versioned_hash) {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // This verifies the proof from a point (x, y) and a commitment
    if !verify_kzg_proof(commitment, x, y, proof).unwrap_or(false) {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // The first 32 bytes consist of the number of field elements in the blob, and the
    // other 32 bytes consist of the modulus used in the BLS signature scheme.
    let output = POINT_EVALUATION_OUTPUT_BYTES.to_vec();

    Ok(Bytes::from(output))
}

#[expect(clippy::indexing_slicing, reason = "slicing bounds checked at start")]
pub fn bls12_g1add(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    // Two inputs of 128 bytes are required
    if calldata.len() != BLS12_381_G1ADD_VALID_INPUT_LENGTH {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // GAS
    increase_precompile_consumed_gas(BLS12_381_G1ADD_COST, gas_remaining)
        .map_err(|_| PrecompileError::NotEnoughGas)?;

    let first_g1_point = parse_g1_point(&calldata[0..128], true)?;
    let second_g1_point = parse_g1_point(&calldata[128..256], true)?;

    let result_of_addition = first_g1_point.add(&second_g1_point);

    if result_of_addition.is_identity().into() {
        return Ok(Bytes::copy_from_slice(&G1_POINT_AT_INFINITY));
    }

    let result_bytes = G1Affine::from(result_of_addition).to_uncompressed();

    let mut padded_result = Vec::with_capacity(128);
    add_padded_coordinate(&mut padded_result, &result_bytes[0..48]);
    add_padded_coordinate(&mut padded_result, &result_bytes[48..96]);

    Ok(Bytes::from(padded_result))
}

/// Signature verification in the “secp256r1” elliptic curve
/// If the verification succeeds, returns 1 in a 32-bit big-endian format.
/// If the verification fails, returns an empty `Bytes` object.
/// Implemented following https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7951.md
pub fn p_256_verify(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    increase_precompile_consumed_gas(P256_VERIFY_COST, gas_remaining)
        .map_err(|_| PrecompileError::NotEnoughGas)?;

    // Validate input data length is 160 bytes
    if calldata.len() != 160 {
        return Ok(Bytes::new());
    }

    // Parse parameters
    #[expect(
        clippy::indexing_slicing,
        reason = "length of the calldata is checked before slicing"
    )]
    let (message_hash, r, s, x, y) = (
        &calldata[0..32],
        &calldata[32..64],
        &calldata[64..96],
        &calldata[96..128],
        &calldata[128..160],
    );

    {
        let [r, s, x, y] = [r, s, x, y].map(P256Uint::from_be_slice);

        // Verify that the r and s values are in (0, n) (exclusive)
        if r == P256Uint::ZERO || r >= P256_N || s == P256Uint::ZERO || s >= P256_N ||
        // Verify that both x and y are in [0, p) (inclusive 0, exclusive p)
        x >= P256_P || y >= P256_P ||
        // Verify that the point (x,y) isn't at infinity
        (x == P256Uint::ZERO && y == P256Uint::ZERO)
        {
            return Ok(Bytes::new());
        }

        // Verify that the point formed by (x, y) is on the curve
        let x: Option<P256FieldElement> = P256FieldElement::from_uint(x).into();
        let y: Option<P256FieldElement> = P256FieldElement::from_uint(y).into();

        let (Some(x), Some(y)) = (x, y) else {
            return Err(InternalError::Slicing.into());
        };

        // Curve equation: `y² = x³ + ax + b`
        let a_x = P256_A.multiply(&x);
        if y.square() != x.pow_vartime(&[3u64]).add(&a_x).add(&P256_B) {
            return Ok(Bytes::new());
        }
    }

    // Build verifier
    let Ok(verifier) = p256::ecdsa::VerifyingKey::from_encoded_point(
        &EncodedPoint::from_affine_coordinates(x.into(), y.into(), false),
    ) else {
        return Ok(Bytes::new());
    };

    // Build signature
    let r: [u8; 32] = r.try_into()?;
    let s: [u8; 32] = s.try_into()?;

    let Ok(signature) = P256Signature::from_scalars(r, s) else {
        return Ok(Bytes::new());
    };

    // Verify message signature
    let success = verifier.verify_prehash(message_hash, &signature).is_ok();

    // If the verification succeeds, returns 1 in a 32-bit big-endian format.
    // If the verification fails, returns an empty `Bytes` object.
    if success {
        const RESULT: [u8; 32] = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1,
        ];
        Ok(Bytes::from_static(&RESULT))
    } else {
        Ok(Bytes::new())
    }
}

pub fn bls12_g1msm(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.is_empty() || calldata.len() % BLS12_381_G1_MSM_PAIR_LENGTH != 0 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let k = calldata.len() / BLS12_381_G1_MSM_PAIR_LENGTH;
    let required_gas = gas_cost::bls12_msm(k, &BLS12_381_G1_K_DISCOUNT, G1_MUL_COST)?;
    increase_precompile_consumed_gas(required_gas, gas_remaining)?;

    let mut result = G1Projective::identity();
    // R = s_P_1 + s_P_2 + ... + s_P_k
    // Where:
    // s_i are scalars (numbers)
    // P_i are points in the group (in this case, points in G1)
    #[expect(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        reason = "bounds checked"
    )]
    for i in 0..k {
        // this operation can't overflow because i < k and  k * BLS12_381_G1_MSM_PAIR_LENGTH = calldata.len()
        let point_offset = i * BLS12_381_G1_MSM_PAIR_LENGTH;
        let scalar_offset = point_offset + 128;
        let pair_end = scalar_offset + 32;

        // slicing is ok because pair_end = point_offset + 160 = (k-1) * 160 + 160 = k * 160 = calldata.len()
        let point = parse_g1_point(&calldata[point_offset..scalar_offset], false)?;
        let scalar = parse_scalar(&calldata[scalar_offset..pair_end])?;

        if !bool::from(scalar.is_zero()) {
            let scaled_point: G1Projective = point * scalar;
            result += scaled_point;
        }
    }
    let mut output = [0u8; 128];

    if result.is_identity().into() {
        return Ok(Bytes::copy_from_slice(&output));
    }

    let result_bytes = G1Affine::from(result).to_uncompressed();
    let (x_bytes, y_bytes) = result_bytes
        .split_at_checked(FIELD_ELEMENT_WITHOUT_PADDING_LENGTH)
        .ok_or(InternalError::Slicing)?;
    output[16..64].copy_from_slice(x_bytes);
    output[80..128].copy_from_slice(y_bytes);

    Ok(Bytes::copy_from_slice(&output))
}

#[expect(clippy::indexing_slicing, reason = "slicing bounds checked at start")]
pub fn bls12_g2add(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.len() != BLS12_381_G2ADD_VALID_INPUT_LENGTH {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // GAS
    increase_precompile_consumed_gas(BLS12_381_G2ADD_COST, gas_remaining)
        .map_err(|_| PrecompileError::NotEnoughGas)?;

    // slices are ok because the len has been validated
    let first_g2_point = parse_g2_point(&calldata[0..256], true)?;
    let second_g2_point = parse_g2_point(&calldata[256..512], true)?;

    let result_of_addition = first_g2_point.add(&second_g2_point);

    if result_of_addition.is_identity().into() {
        return Ok(Bytes::copy_from_slice(&G2_POINT_AT_INFINITY));
    }

    let result_bytes = G2Affine::from(result_of_addition).to_uncompressed();

    let mut padded_result = Vec::with_capacity(256);
    // The crate bls12_381 deserialize the G2 point as x_1 || x_0 || y_1 || y_0
    // https://docs.rs/bls12_381/0.8.0/src/bls12_381/g2.rs.html#284-299
    add_padded_coordinate(&mut padded_result, &result_bytes[48..96]);
    add_padded_coordinate(&mut padded_result, &result_bytes[0..48]);
    add_padded_coordinate(&mut padded_result, &result_bytes[144..192]);
    add_padded_coordinate(&mut padded_result, &result_bytes[96..144]);

    Ok(Bytes::from(padded_result))
}

pub fn bls12_g2msm(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.is_empty() || calldata.len() % BLS12_381_G2_MSM_PAIR_LENGTH != 0 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let k = calldata.len() / BLS12_381_G2_MSM_PAIR_LENGTH;
    let required_gas = gas_cost::bls12_msm(k, &BLS12_381_G2_K_DISCOUNT, G2_MUL_COST)?;
    increase_precompile_consumed_gas(required_gas, gas_remaining)?;

    let mut result = G2Projective::identity();

    #[expect(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        reason = "bounds checked"
    )]
    for i in 0..k {
        // this operation can't overflow because i < k and  k * BLS12_381_G2_MSM_PAIR_LENGTH = calldata.len()
        let point_offset = i * BLS12_381_G2_MSM_PAIR_LENGTH;
        let scalar_offset = point_offset + 256;
        let pair_end = scalar_offset + 32;

        // slicing is ok because at the max value of i,
        // (k-1) * BLS12_381_G2_MSM_PAIR_LENGTH + 256 ≤ k * BLS12_381_G2_MSM_PAIR_LENGTH
        let point = parse_g2_point(&calldata[point_offset..scalar_offset], false)?;
        let scalar = parse_scalar(&calldata[scalar_offset..pair_end])?;

        // skip zero scalars
        if scalar != Scalar::zero() {
            let scaled_point: G2Projective = point * scalar;
            result += scaled_point;
        }
    }

    let result_bytes = if result.is_identity().into() {
        return Ok(Bytes::copy_from_slice(&G2_POINT_AT_INFINITY));
    } else {
        G2Affine::from(result).to_uncompressed()
    };

    let mut padded_result = Vec::with_capacity(256);
    // The crate bls12_381 deserialize the G2 point as x_1 || x_0 || y_1 || y_0
    // https://docs.rs/bls12_381/0.8.0/src/bls12_381/g2.rs.html#284-299
    add_padded_coordinate(&mut padded_result, &result_bytes[48..96]);
    add_padded_coordinate(&mut padded_result, &result_bytes[0..48]);
    add_padded_coordinate(&mut padded_result, &result_bytes[144..192]);
    add_padded_coordinate(&mut padded_result, &result_bytes[96..144]);

    Ok(Bytes::from(padded_result))
}

pub fn bls12_pairing_check(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.is_empty() || calldata.len() % BLS12_381_PAIRING_CHECK_PAIR_LENGTH != 0 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // GAS
    let k = calldata.len() / BLS12_381_PAIRING_CHECK_PAIR_LENGTH;
    let gas_cost = gas_cost::bls12_pairing_check(k)?;
    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let mut points: Vec<(G1Affine, G2Prepared)> = Vec::new();
    #[expect(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        reason = "bounds checked"
    )]
    for i in 0..k {
        let g1_point_offset = i * BLS12_381_PAIRING_CHECK_PAIR_LENGTH;
        let g2_point_offset = g1_point_offset + 128;
        let pair_end = g2_point_offset + 256;

        // The check for the subgroup is required
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2537.md?plain=1#L194
        let g1 = G1Affine::from(parse_g1_point(
            &calldata[g1_point_offset..g2_point_offset],
            false,
        )?);
        let g2 = G2Affine::from(parse_g2_point(&calldata[g2_point_offset..pair_end], false)?);
        points.push((g1, G2Prepared::from(g2)));
    }

    // The crate bls12_381 expects a reference to the points
    let points: Vec<(&G1Affine, &G2Prepared)> = points.iter().map(|(g1, g2)| (g1, g2)).collect();

    // perform the final exponentiation to get the result of the pairing check
    // https://docs.rs/bls12_381/0.8.0/src/bls12_381/pairings.rs.html#43-48
    let result: Gt = multi_miller_loop(&points).final_exponentiation();

    // follows this https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2537.md?plain=1#L188
    if result == Gt::identity() {
        let mut result = vec![0_u8; 31];
        result.push(1);
        Ok(Bytes::from(result))
    } else {
        Ok(Bytes::copy_from_slice(&[0_u8; 32]))
    }
}

pub fn bls12_map_fp_to_g1(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.len() != BLS12_381_FP_VALID_INPUT_LENGTH {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // GAS
    increase_precompile_consumed_gas(BLS12_381_MAP_FP_TO_G1_COST, gas_remaining)?;

    // PADDED_FIELD_ELEMENT_SIZE_IN_BYTES == BLS12_381_FP_VALID_INPUT_LENGTH, so this slice is ok.
    #[expect(clippy::indexing_slicing, reason = "bounds checked")]
    let coordinate_bytes = parse_coordinate(&calldata[0..PADDED_FIELD_ELEMENT_SIZE_IN_BYTES])?;
    let fp = Fp::from_bytes(&coordinate_bytes)
        .into_option()
        .ok_or(ExceptionalHalt::Precompile(
            PrecompileError::ParsingInputError,
        ))?;

    // following https://github.com/ethereum/EIPs/blob/master/assets/eip-2537/field_to_curve.md?plain=1#L3-L6, we do:
    // map_to_curve: map a field element to a another curve, then isogeny is applied to map to the curve bls12_381
    // clear_h: clears the cofactor
    let point = G1Projective::map_to_curve(&fp).clear_h();

    let result_bytes = if point.is_identity().into() {
        return Ok(Bytes::copy_from_slice(&G1_POINT_AT_INFINITY));
    } else {
        G1Affine::from(point).to_uncompressed()
    };

    let mut padded_result = Vec::with_capacity(128);
    add_padded_coordinate(&mut padded_result, &result_bytes[0..48]);
    add_padded_coordinate(&mut padded_result, &result_bytes[48..96]);

    Ok(Bytes::from(padded_result))
}

pub fn bls12_map_fp2_tp_g2(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.len() != BLS12_381_FP2_VALID_INPUT_LENGTH {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // GAS
    increase_precompile_consumed_gas(BLS12_381_MAP_FP2_TO_G2_COST, gas_remaining)?;

    // slices are ok because of the previous len check.
    // Parse the input to two Fp and create a Fp2
    #[expect(clippy::indexing_slicing, reason = "bounds checked")]
    let c0 = parse_coordinate(&calldata[0..PADDED_FIELD_ELEMENT_SIZE_IN_BYTES])?;
    #[expect(clippy::indexing_slicing, reason = "bounds checked")]
    let c1 = parse_coordinate(
        &calldata[PADDED_FIELD_ELEMENT_SIZE_IN_BYTES..BLS12_381_FP2_VALID_INPUT_LENGTH],
    )?;
    let fp_0 = Fp::from_bytes(&c0)
        .into_option()
        .ok_or(ExceptionalHalt::Precompile(
            PrecompileError::ParsingInputError,
        ))?;
    let fp_1 = Fp::from_bytes(&c1)
        .into_option()
        .ok_or(ExceptionalHalt::Precompile(
            PrecompileError::ParsingInputError,
        ))?;
    if fp_0 == Fp::zero() && fp_1 == Fp::zero() {
        return Ok(Bytes::copy_from_slice(&FP2_ZERO_MAPPED_TO_G2));
    }

    let fp2 = Fp2 { c0: fp_0, c1: fp_1 };

    // following https://github.com/ethereum/EIPs/blob/master/assets/eip-2537/field_to_curve.md?plain=1#L3-L6, we do:
    // map_to_curve: map a field element to a another curve, then isogeny is applied to map to the curve bls12_381
    // clear_h: clears the cofactor
    let point = G2Projective::map_to_curve(&fp2).clear_h();
    let result_bytes = if point.is_identity().into() {
        return Ok(Bytes::copy_from_slice(&G2_POINT_AT_INFINITY));
    } else {
        G2Affine::from(point).to_uncompressed()
    };

    let mut padded_result = Vec::with_capacity(256);
    // The crate bls12_381 deserialize the G2 point as x_1 || x_0 || y_1 || y_0
    // https://docs.rs/bls12_381/0.8.0/src/bls12_381/g2.rs.html#284-299
    add_padded_coordinate(&mut padded_result, &result_bytes[48..96]);
    add_padded_coordinate(&mut padded_result, &result_bytes[0..48]);
    add_padded_coordinate(&mut padded_result, &result_bytes[144..192]);
    add_padded_coordinate(&mut padded_result, &result_bytes[96..144]);

    Ok(Bytes::from(padded_result))
}

/// coordinate raw bytes should have a len of 64
#[expect(clippy::indexing_slicing, reason = "bounds checked at start")]
#[inline]
fn parse_coordinate(coordinate_raw_bytes: &[u8]) -> Result<[u8; 48], VMError> {
    const SIXTEEN_ZEROES: [u8; 16] = [0; 16];

    if coordinate_raw_bytes.len() != 64 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    if coordinate_raw_bytes[0..16] != SIXTEEN_ZEROES {
        return Err(PrecompileError::ParsingInputError.into());
    }

    #[expect(
        unsafe_code,
        reason = "The bounds are confirmed to be correct due to the previous checks."
    )]
    unsafe {
        Ok(coordinate_raw_bytes[16..64].try_into().unwrap_unchecked())
    }
}

/// point_bytes must have atleast 128 bytes.
#[expect(clippy::indexing_slicing, reason = "slice bounds checked at start")]
fn parse_g1_point(point_bytes: &[u8], unchecked: bool) -> Result<G1Projective, VMError> {
    if point_bytes.len() != 128 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let x = parse_coordinate(&point_bytes[0..64])?;
    let y = parse_coordinate(&point_bytes[64..128])?;

    const ALL_ZERO: [u8; 48] = [0; 48];

    // if a g1 point decode to (0,0) by convention it is interpreted as a point to infinity
    let g1_point: G1Projective = if x == ALL_ZERO && y == ALL_ZERO {
        G1Projective::identity()
    } else {
        let g1_bytes: [u8; 96] = [x, y]
            .concat()
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?;

        if unchecked {
            // We use unchecked because in the https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2537.md?plain=1#L141
            // note that there is no subgroup check for the G1 addition precompile
            let g1_affine = G1Affine::from_uncompressed_unchecked(&g1_bytes)
                .into_option()
                .ok_or(ExceptionalHalt::Precompile(
                    PrecompileError::ParsingInputError,
                ))?;

            // We still need to check if the point is on the curve
            if !bool::from(g1_affine.is_on_curve()) {
                return Err(ExceptionalHalt::Precompile(
                    PrecompileError::BLS12381G1PointNotInCurve,
                )
                .into());
            }

            G1Projective::from(g1_affine)
        } else {
            let g1_affine = G1Affine::from_uncompressed(&g1_bytes)
                .into_option()
                .ok_or(PrecompileError::ParsingInputError)?;

            G1Projective::from(g1_affine)
        }
    };
    Ok(g1_point)
}

/// point_bytes always has atleast 256 bytes
#[expect(clippy::indexing_slicing, reason = "slice bounds checked at start")]
fn parse_g2_point(point_bytes: &[u8], unchecked: bool) -> Result<G2Projective, VMError> {
    if point_bytes.len() != 256 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    const ALL_ZERO: [u8; 48] = [0; 48];

    let x_0 = parse_coordinate(&point_bytes[0..64])?;
    let x_1 = parse_coordinate(&point_bytes[64..128])?;
    let y_0 = parse_coordinate(&point_bytes[128..192])?;
    let y_1 = parse_coordinate(&point_bytes[192..256])?;

    // if a g1 point decode to (0,0) by convention it is interpreted as a point to infinity
    let g2_point: G2Projective =
        if x_0 == ALL_ZERO && x_1 == ALL_ZERO && y_0 == ALL_ZERO && y_1 == ALL_ZERO {
            G2Projective::identity()
        } else {
            // The crate serialize the coordinates in a reverse order
            // https://docs.rs/bls12_381/0.8.0/src/bls12_381/g2.rs.html#401-464
            let mut g2_bytes: [u8; 192] = [0; 192];
            g2_bytes[0..48].copy_from_slice(&x_1);
            g2_bytes[48..96].copy_from_slice(&x_0);
            g2_bytes[96..144].copy_from_slice(&y_1);
            g2_bytes[144..192].copy_from_slice(&y_0);

            if unchecked {
                // We use unchecked because in the https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2537.md?plain=1#L141
                // note that there is no subgroup check for the G1 addition precompile
                let g2_affine = G2Affine::from_uncompressed_unchecked(&g2_bytes)
                    .into_option()
                    .ok_or(ExceptionalHalt::Precompile(
                        PrecompileError::ParsingInputError,
                    ))?;

                // We still need to check if the point is on the curve
                if !bool::from(g2_affine.is_on_curve()) {
                    return Err(ExceptionalHalt::Precompile(
                        PrecompileError::BLS12381G2PointNotInCurve,
                    )
                    .into());
                }

                G2Projective::from(g2_affine)
            } else {
                let g2_affine = G2Affine::from_uncompressed(&g2_bytes)
                    .into_option()
                    .ok_or(PrecompileError::ParsingInputError)?;

                G2Projective::from(g2_affine)
            }
        };
    Ok(g2_point)
}

// coordinate_raw_bytes usually has 48 bytes
#[inline]
fn add_padded_coordinate(result: &mut Vec<u8>, coordinate_raw_bytes: &[u8]) {
    // add the padding to satisfy the convention of encoding
    // https://eips.ethereum.org/EIPS/eip-2537
    const SIXTEEN_ZEROES: [u8; 16] = [0; 16];
    result.reserve(16 + 48);
    result.extend_from_slice(&SIXTEEN_ZEROES);
    result.extend_from_slice(coordinate_raw_bytes);
}

#[allow(clippy::indexing_slicing, reason = "bounds checked at start")]
#[inline]
fn parse_scalar(scalar_bytes: &[u8]) -> Result<Scalar, VMError> {
    if scalar_bytes.len() != 32 {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let scalar_le = [
        u64::from_be_bytes([
            scalar_bytes[24],
            scalar_bytes[25],
            scalar_bytes[26],
            scalar_bytes[27],
            scalar_bytes[28],
            scalar_bytes[29],
            scalar_bytes[30],
            scalar_bytes[31],
        ]),
        u64::from_be_bytes([
            scalar_bytes[16],
            scalar_bytes[17],
            scalar_bytes[18],
            scalar_bytes[19],
            scalar_bytes[20],
            scalar_bytes[21],
            scalar_bytes[22],
            scalar_bytes[23],
        ]),
        u64::from_be_bytes([
            scalar_bytes[8],
            scalar_bytes[9],
            scalar_bytes[10],
            scalar_bytes[11],
            scalar_bytes[12],
            scalar_bytes[13],
            scalar_bytes[14],
            scalar_bytes[15],
        ]),
        u64::from_be_bytes([
            scalar_bytes[0],
            scalar_bytes[1],
            scalar_bytes[2],
            scalar_bytes[3],
            scalar_bytes[4],
            scalar_bytes[5],
            scalar_bytes[6],
            scalar_bytes[7],
        ]),
    ];
    Ok(Scalar::from_raw(scalar_le))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ec_pairing(calldata: &str, expected_output: &str, mut gas: u64) {
        let calldata = Bytes::from(hex::decode(calldata).unwrap());
        let expected_output = Bytes::from(hex::decode(expected_output).unwrap());
        let output = ecpairing(&calldata, &mut gas, Fork::Cancun).unwrap();
        assert_eq!(output, expected_output);
        assert!(gas.is_zero());
    }

    // ec pairing precompile test data taken from https://github.com/ethereum/go-ethereum/blob/master/core/vm/testdata/precompiles/bn256Pairing.json

    #[test]
    fn test_ec_pairing_a() {
        test_ec_pairing(
            "1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f593034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf704bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a416782bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c2032c61a830e3c17286de9462bf242fca2883585b93870a73853face6a6bf411198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "0000000000000000000000000000000000000000000000000000000000000001",
            113000,
        );
    }

    #[test]
    fn test_ec_pairing_b() {
        test_ec_pairing(
            "2eca0c7238bf16e83e7a1e6c5d49540685ff51380f309842a98561558019fc0203d3260361bb8451de5ff5ecd17f010ff22f5c31cdf184e9020b06fa5997db841213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f06967a1237ebfeca9aaae0d6d0bab8e28c198c5a339ef8a2407e31cdac516db922160fa257a5fd5b280642ff47b65eca77e626cb685c84fa6d3b6882a283ddd1198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "0000000000000000000000000000000000000000000000000000000000000001",
            113000,
        );
    }
    #[test]
    fn test_ec_pairing_c() {
        test_ec_pairing(
            "0f25929bcb43d5a57391564615c9e70a992b10eafa4db109709649cf48c50dd216da2f5cb6be7a0aa72c440c53c9bbdfec6c36c7d515536431b3a865468acbba2e89718ad33c8bed92e210e81d1853435399a271913a6520736a4729cf0d51eb01a9e2ffa2e92599b68e44de5bcf354fa2642bd4f26b259daa6f7ce3ed57aeb314a9a87b789a58af499b314e13c3d65bede56c07ea2d418d6874857b70763713178fb49a2d6cd347dc58973ff49613a20757d0fcc22079f9abd10c3baee245901b9e027bd5cfc2cb5db82d4dc9677ac795ec500ecd47deee3b5da006d6d049b811d7511c78158de484232fc68daf8a45cf217d1c2fae693ff5871e8752d73b21198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "0000000000000000000000000000000000000000000000000000000000000001",
            113000,
        )
    }

    #[test]
    fn test_ec_pairing_d() {
        test_ec_pairing(
            "2f2ea0b3da1e8ef11914acf8b2e1b32d99df51f5f4f206fc6b947eae860eddb6068134ddb33dc888ef446b648d72338684d678d2eb2371c61a50734d78da4b7225f83c8b6ab9de74e7da488ef02645c5a16a6652c3c71a15dc37fe3a5dcb7cb122acdedd6308e3bb230d226d16a105295f523a8a02bfc5e8bd2da135ac4c245d065bbad92e7c4e31bf3757f1fe7362a63fbfee50e7dc68da116e67d600d9bf6806d302580dc0661002994e7cd3a7f224e7ddc27802777486bf80f40e4ca3cfdb186bac5188a98c45e6016873d107f5cd131f3a3e339d0375e58bd6219347b008122ae2b09e539e152ec5364e7e2204b03d11d3caa038bfc7cd499f8176aacbee1f39e4e4afc4bc74790a4a028aff2c3d2538731fb755edefd8cb48d6ea589b5e283f150794b6736f670d6a1033f9b46c6f5204f50813eb85c8dc4b59db1c5d39140d97ee4d2b36d99bc49974d18ecca3e7ad51011956051b464d9e27d46cc25e0764bb98575bd466d32db7b15f582b2d5c452b36aa394b789366e5e3ca5aabd415794ab061441e51d01e94640b7e3084a07e02c78cf3103c542bc5b298669f211b88da1679b0b64a63b7e0e7bfe52aae524f73a55be7fe70c7e9bfc94b4cf0da1213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f",
            "0000000000000000000000000000000000000000000000000000000000000001",
            147000,
        )
    }

    #[test]
    fn test_ec_pairing_e() {
        test_ec_pairing(
            "20a754d2071d4d53903e3b31a7e98ad6882d58aec240ef981fdf0a9d22c5926a29c853fcea789887315916bbeb89ca37edb355b4f980c9a12a94f30deeed30211213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f1abb4a25eb9379ae96c84fff9f0540abcfc0a0d11aeda02d4f37e4baf74cb0c11073b3ff2cdbb38755f8691ea59e9606696b3ff278acfc098fa8226470d03869217cee0a9ad79a4493b5253e2e4e3a39fc2df38419f230d341f60cb064a0ac290a3d76f140db8418ba512272381446eb73958670f00cf46f1d9e64cba057b53c26f64a8ec70387a13e41430ed3ee4a7db2059cc5fc13c067194bcc0cb49a98552fd72bd9edb657346127da132e5b82ab908f5816c826acb499e22f2412d1a2d70f25929bcb43d5a57391564615c9e70a992b10eafa4db109709649cf48c50dd2198a1f162a73261f112401aa2db79c7dab1533c9935c77290a6ce3b191f2318d198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "0000000000000000000000000000000000000000000000000000000000000001",
            147000,
        )
    }

    #[test]
    fn test_ec_pairing_f() {
        test_ec_pairing(
            "1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f593034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf704bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a416782bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c103188585e2364128fe25c70558f1560f4f9350baf3959e603cc91486e110936198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "0000000000000000000000000000000000000000000000000000000000000000",
            113000,
        )
    }

    #[test]
    fn test_ec_pairing_g() {
        test_ec_pairing(
            "",
            "0000000000000000000000000000000000000000000000000000000000000001",
            45000,
        )
    }

    #[test]
    fn test_ec_pairing_h() {
        test_ec_pairing(
            "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "0000000000000000000000000000000000000000000000000000000000000000",
            79000,
        )
    }

    #[test]
    fn test_ec_pairing_i() {
        test_ec_pairing(
            "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d",
            "0000000000000000000000000000000000000000000000000000000000000001",
            113000,
        )
    }

    #[test]
    fn test_ec_pairing_j() {
        test_ec_pairing(
            "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "0000000000000000000000000000000000000000000000000000000000000001",
            113000,
        )
    }

    #[test]
    fn test_ec_pairing_k() {
        test_ec_pairing(
            "105456a333e6d636854f987ea7bb713dfd0ae8371a72aea313ae0c32c0bf10160cf031d41b41557f3e7e3ba0c51bebe5da8e6ecd855ec50fc87efcdeac168bcc0476be093a6d2b4bbf907172049874af11e1b6267606e00804d3ff0037ec57fd3010c68cb50161b7d1d96bb71edfec9880171954e56871abf3d93cc94d745fa114c059d74e5b6c4ec14ae5864ebe23a71781d86c29fb8fb6cce94f70d3de7a2101b33461f39d9e887dbb100f170a2345dde3c07e256d1dfa2b657ba5cd030427000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000021a2c3013d2ea92e13c800cde68ef56a294b883f6ac35d25f587c09b1b3c635f7290158a80cd3d66530f74dc94c94adb88f5cdb481acca997b6e60071f08a115f2f997f3dbd66a7afe07fe7862ce239edba9e05c5afff7f8a1259c9733b2dfbb929d1691530ca701b4a106054688728c9972c8512e9789e9567aae23e302ccd75",
            "0000000000000000000000000000000000000000000000000000000000000001",
            113000,
        )
    }

    #[test]
    fn test_ec_pairing_l() {
        test_ec_pairing(
            "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d",
            "0000000000000000000000000000000000000000000000000000000000000001",
            385000,
        )
    }

    #[test]
    fn test_ec_pairing_m() {
        test_ec_pairing(
            "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "0000000000000000000000000000000000000000000000000000000000000001",
            385000,
        )
    }

    #[test]
    fn test_ec_pairing_n() {
        test_ec_pairing(
            "105456a333e6d636854f987ea7bb713dfd0ae8371a72aea313ae0c32c0bf10160cf031d41b41557f3e7e3ba0c51bebe5da8e6ecd855ec50fc87efcdeac168bcc0476be093a6d2b4bbf907172049874af11e1b6267606e00804d3ff0037ec57fd3010c68cb50161b7d1d96bb71edfec9880171954e56871abf3d93cc94d745fa114c059d74e5b6c4ec14ae5864ebe23a71781d86c29fb8fb6cce94f70d3de7a2101b33461f39d9e887dbb100f170a2345dde3c07e256d1dfa2b657ba5cd030427000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000021a2c3013d2ea92e13c800cde68ef56a294b883f6ac35d25f587c09b1b3c635f7290158a80cd3d66530f74dc94c94adb88f5cdb481acca997b6e60071f08a115f2f997f3dbd66a7afe07fe7862ce239edba9e05c5afff7f8a1259c9733b2dfbb929d1691530ca701b4a106054688728c9972c8512e9789e9567aae23e302ccd75",
            "0000000000000000000000000000000000000000000000000000000000000001",
            113000,
        )
    }

    #[test]
    // Calldata taken from failed transaction https://sepolia.etherscan.io/tx/0x4355d49be46e61a53c71f45a128ebefb52cb38df08ed55833c2c162d26396819
    fn test_ec_pairing_coordinate_out_of_bounds() {
        let calldata = Bytes::from(hex::decode("30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd4830644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd49198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa").unwrap());
        let mut gas_remaining = u64::MAX;
        assert_eq!(
            ecpairing(&calldata, &mut gas_remaining, Fork::Cancun),
            Err(PrecompileError::CoordinateExceedsFieldModulus.into())
        );
    }
}
