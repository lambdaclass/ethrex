use ark_bn254::{Fr as FrArk, G1Affine as G1AffineArk};
use ark_ec::CurveGroup;
use ark_ff::{BigInteger, PrimeField as ArkPrimeField, Zero};
use bls12_381::{
    Fp, Fp2, G1Affine, G1Projective, G2Affine, G2Prepared, G2Projective, Gt, Scalar,
    hash_to_curve::MapToCurve, multi_miller_loop,
};
use bytes::{Buf, Bytes};
use ethrex_common::H160;
use ethrex_common::utils::u256_from_big_endian_const;
use ethrex_common::{
    Address, H256, U256, serde_utils::bool, types::Fork, types::Fork::*,
    utils::u256_from_big_endian,
};
use ethrex_crypto::{blake2f::blake2b_f, kzg::verify_kzg_proof};
use k256::elliptic_curve::Field;
use lambdaworks_math::{
    elliptic_curve::short_weierstrass::curves::bls12_381::{
        curve::BLS12381TwistCurveFieldElement, twist::BLS12381TwistCurve,
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
use rustc_hash::FxHashMap;
use sha2::Digest;
use std::borrow::Cow;
use std::ops::Mul;
use std::sync::RwLock;

use crate::constants::{P256_A, P256_B, P256_N};
use crate::gas_cost::{MODEXP_STATIC_COST, P256_VERIFY_COST};
use crate::vm::VMType;
use crate::{
    constants::{P256_P, VERSIONED_HASH_VERSION_KZG},
    errors::{ExceptionalHalt, InternalError, PrecompileError, VMError},
    gas_cost::{
        self, BLAKE2F_ROUND_COST, BLS12_381_G1_K_DISCOUNT, BLS12_381_G1ADD_COST,
        BLS12_381_G2_K_DISCOUNT, BLS12_381_G2ADD_COST, BLS12_381_MAP_FP_TO_G1_COST,
        BLS12_381_MAP_FP2_TO_G2_COST, ECADD_COST, ECMUL_COST, G1_MUL_COST, G2_MUL_COST,
        POINT_EVALUATION_COST,
    },
};
use lambdaworks_math::elliptic_curve::short_weierstrass::curves::bls12_381::curve::{
    BLS12381Curve, BLS12381FieldElement,
};
use lambdaworks_math::elliptic_curve::short_weierstrass::curves::bls12_381::field_extension::BLS12381FieldModulus;
use lambdaworks_math::elliptic_curve::short_weierstrass::traits::IsShortWeierstrass;
use lambdaworks_math::field::fields::montgomery_backed_prime_fields::IsModulus;

pub const BLAKE2F_ELEMENT_SIZE: usize = 8;

pub const SIZE_PRECOMPILES_PRE_CANCUN: u64 = 9;
pub const SIZE_PRECOMPILES_CANCUN: u64 = 10;
pub const SIZE_PRECOMPILES_PRAGUE: u64 = 17;

pub const BLS12_381_G1_MSM_PAIR_LENGTH: usize = 160;
pub const BLS12_381_G2_MSM_PAIR_LENGTH: usize = 288;
pub const BLS12_381_PAIRING_CHECK_PAIR_LENGTH: usize = 384;

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

pub struct Precompile {
    pub address: H160,
    pub name: &'static str,
    pub active_since_fork: Fork,
}

pub const ECRECOVER: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01,
    ]),
    name: "ECREC",
    active_since_fork: Paris,
};

pub const SHA2_256: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x02,
    ]),
    name: "SHA256",
    active_since_fork: Paris,
};

pub const RIPEMD_160: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x03,
    ]),
    name: "RIPEMD160",
    active_since_fork: Paris,
};

pub const IDENTITY: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x04,
    ]),
    name: "ID",
    active_since_fork: Paris,
};

pub const MODEXP: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x05,
    ]),
    name: "MODEXP",
    active_since_fork: Paris,
};

pub const ECADD: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x06,
    ]),
    name: "BN254_ADD",
    active_since_fork: Paris,
};

pub const ECMUL: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x07,
    ]),
    name: "BN254_MUL",
    active_since_fork: Paris,
};

pub const ECPAIRING: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x08,
    ]),
    name: "BN254_PAIRING",
    active_since_fork: Paris,
};

pub const BLAKE2F: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x09,
    ]),
    name: "BLAKE2F",
    active_since_fork: Paris,
};

pub const POINT_EVALUATION: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x0a,
    ]),
    name: "KZG_POINT_EVALUATION",
    active_since_fork: Cancun,
};

pub const BLS12_G1ADD: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x0b,
    ]),
    name: "BLS12_G1ADD",
    active_since_fork: Prague,
};

pub const BLS12_G1MSM: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x0c,
    ]),
    name: "BLS12_G1MSM",
    active_since_fork: Prague,
};

pub const BLS12_G2ADD: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x0d,
    ]),
    name: "BLS12_G2ADD",
    active_since_fork: Prague,
};

pub const BLS12_G2MSM: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x0e,
    ]),
    name: "BLS12_G2MSM",
    active_since_fork: Prague,
};

pub const BLS12_PAIRING_CHECK: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x0f,
    ]),
    name: "BLS12_PAIRING_CHECK",
    active_since_fork: Prague,
};

pub const BLS12_MAP_FP_TO_G1: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x10,
    ]),
    name: "BLS12_MAP_FP_TO_G1",
    active_since_fork: Prague,
};

pub const BLS12_MAP_FP2_TO_G2: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x11,
    ]),
    name: "BLS12_MAP_FP2_TO_G2",
    active_since_fork: Prague,
};

pub const P256VERIFY: Precompile = Precompile {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x01, 0x00,
    ]),
    name: "P256VERIFY",
    active_since_fork: Osaka,
};

pub const PRECOMPILES: [Precompile; 19] = [
    ECRECOVER,
    SHA2_256,
    RIPEMD_160,
    IDENTITY,
    MODEXP,
    ECADD,
    ECMUL,
    ECPAIRING,
    BLAKE2F,
    POINT_EVALUATION,
    BLS12_G1ADD,
    BLS12_G1MSM,
    BLS12_G2ADD,
    BLS12_G2MSM,
    BLS12_MAP_FP_TO_G1,
    BLS12_MAP_FP2_TO_G2,
    BLS12_MAP_FP_TO_G1,
    BLS12_PAIRING_CHECK,
    P256VERIFY,
];

pub fn precompiles_for_fork(fork: Fork) -> impl Iterator<Item = Precompile> {
    PRECOMPILES
        .into_iter()
        .filter(move |precompile| precompile.active_since_fork <= fork)
}

pub fn is_precompile(address: &Address, fork: Fork, vm_type: VMType) -> bool {
    (matches!(vm_type, VMType::L2(_)) && *address == P256VERIFY.address)
        || precompiles_for_fork(fork).any(|precompile| precompile.address == *address)
}

/// Per-block cache for precompile results shared between warmer and executor.
pub struct PrecompileCache {
    cache: RwLock<FxHashMap<(Address, Bytes), (Bytes, u64)>>,
}

impl Default for PrecompileCache {
    fn default() -> Self {
        Self {
            cache: RwLock::new(FxHashMap::default()),
        }
    }
}

impl PrecompileCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, address: &Address, calldata: &Bytes) -> Option<(Bytes, u64)> {
        // Graceful degradation: if the lock is poisoned (a thread panicked while
        // holding it), skip the cache rather than propagating the panic. The cache
        // is a pure optimization — missing it only costs a recomputation.
        self.cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(*address, calldata.clone()))
            .cloned()
    }

    pub fn insert(&self, address: Address, calldata: Bytes, output: Bytes, gas_cost: u64) {
        self.cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((address, calldata), (output, gas_cost));
    }
}

#[expect(clippy::as_conversions, clippy::indexing_slicing)]
pub fn execute_precompile(
    address: Address,
    calldata: &Bytes,
    gas_remaining: &mut u64,
    fork: Fork,
    cache: Option<&PrecompileCache>,
) -> Result<Bytes, VMError> {
    type PrecompileFn = fn(&Bytes, &mut u64, Fork) -> Result<Bytes, VMError>;

    const PRECOMPILES: [Option<PrecompileFn>; 512] = const {
        let mut precompiles = [const { None }; 512];
        precompiles[ECRECOVER.address.0[19] as usize] = Some(ecrecover as PrecompileFn);
        precompiles[IDENTITY.address.0[19] as usize] = Some(identity as PrecompileFn);
        precompiles[SHA2_256.address.0[19] as usize] = Some(sha2_256 as PrecompileFn);
        precompiles[RIPEMD_160.address.0[19] as usize] = Some(ripemd_160 as PrecompileFn);
        precompiles[MODEXP.address.0[19] as usize] = Some(modexp as PrecompileFn);
        precompiles[ECADD.address.0[19] as usize] = Some(ecadd as PrecompileFn);
        precompiles[ECMUL.address.0[19] as usize] = Some(ecmul as PrecompileFn);
        precompiles[ECPAIRING.address.0[19] as usize] = Some(ecpairing as PrecompileFn);
        precompiles[BLAKE2F.address.0[19] as usize] = Some(blake2f as PrecompileFn);
        precompiles[POINT_EVALUATION.address.0[19] as usize] =
            Some(point_evaluation as PrecompileFn);
        precompiles[BLS12_G1ADD.address.0[19] as usize] = Some(bls12_g1add as PrecompileFn);
        precompiles[BLS12_G1MSM.address.0[19] as usize] = Some(bls12_g1msm as PrecompileFn);
        precompiles[BLS12_G2ADD.address.0[19] as usize] = Some(bls12_g2add as PrecompileFn);
        precompiles[BLS12_G2MSM.address.0[19] as usize] = Some(bls12_g2msm as PrecompileFn);
        precompiles[BLS12_PAIRING_CHECK.address.0[19] as usize] =
            Some(bls12_pairing_check as PrecompileFn);
        precompiles[BLS12_MAP_FP_TO_G1.address.0[19] as usize] =
            Some(bls12_map_fp_to_g1 as PrecompileFn);
        precompiles[BLS12_MAP_FP2_TO_G2.address.0[19] as usize] =
            Some(bls12_map_fp2_tp_g2 as PrecompileFn);
        precompiles
            [u16::from_be_bytes([P256VERIFY.address.0[18], P256VERIFY.address.0[19]]) as usize] =
            Some(p_256_verify as PrecompileFn);
        precompiles
    };

    if address[0..18] != [0u8; 18] {
        return Err(VMError::Internal(InternalError::InvalidPrecompileAddress));
    }
    let index = u16::from_be_bytes([address[18], address[19]]) as usize;

    let precompile = PRECOMPILES
        .get(index)
        .copied()
        .flatten()
        .ok_or(VMError::Internal(InternalError::InvalidPrecompileAddress))?;

    // Check cache (skip identity -- copy is cheaper than lookup)
    if address != IDENTITY.address
        && let Some((output, gas_cost)) = cache.and_then(|c| c.get(&address, calldata))
    {
        increase_precompile_consumed_gas(gas_cost, gas_remaining)?;
        return Ok(output);
    }

    #[cfg(feature = "perf_opcode_timings")]
    let precompile_time_start = std::time::Instant::now();

    let gas_before = *gas_remaining;
    let result = precompile(calldata, gas_remaining, fork);

    #[cfg(feature = "perf_opcode_timings")]
    {
        let time = precompile_time_start.elapsed();
        let mut timings = crate::timings::PRECOMPILES_TIMINGS.lock().expect("poison");
        timings.update(address, time);
    }

    // Cache result on success (skip identity)
    if address != IDENTITY.address
        && let Some(cache) = cache
        && let Ok(output) = &result
    {
        let gas_cost = gas_before.saturating_sub(*gas_remaining);
        cache.insert(address, calldata.clone(), output.clone(), gas_cost);
    }

    result
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

#[expect(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#[cfg(all(
    not(feature = "sp1"),
    not(feature = "risc0"),
    not(feature = "zisk"),
    feature = "secp256k1"
))]
#[inline(always)]
fn copy_segment(calldata: &Bytes, dst: &mut [u8], start: usize) {
    if start >= calldata.len() {
        return;
    }
    let end = (start + dst.len()).min(calldata.len());
    let src = &calldata[start..end];
    dst[..src.len()].copy_from_slice(src);
}

#[cfg(all(
    not(feature = "sp1"),
    not(feature = "risc0"),
    not(feature = "zisk"),
    feature = "secp256k1"
))]
pub fn ecrecover(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    use crate::gas_cost::ECRECOVER_COST;

    increase_precompile_consumed_gas(ECRECOVER_COST, gas_remaining)?;

    const WORD: usize = 32;
    const SIG_LEN: usize = 64;

    // Total input size = 128
    let mut raw_hash = [0u8; WORD];
    let mut raw_v = [0u8; WORD];
    let mut raw_sig = [0u8; SIG_LEN];

    copy_segment(calldata, &mut raw_hash, 0);
    copy_segment(calldata, &mut raw_v, WORD);
    copy_segment(calldata, &mut raw_sig, WORD * 2);

    // EVM expects v ∈ {27, 28}. Anything else is invalid → empty return.
    if raw_v[..(WORD - 1)].iter().any(|&b| b != 0) {
        return Ok(Bytes::new());
    }
    let recovery_id_byte = match raw_v[WORD - 1] {
        27 => 0_i32,
        28 => 1_i32,
        _ => return Ok(Bytes::new()),
    };

    // Recovery id from the adjusted byte.
    let Ok(recovery_id) = secp256k1::ecdsa::RecoveryId::try_from(recovery_id_byte) else {
        return Ok(Bytes::new());
    };

    let Ok(recoverable_signature) =
        secp256k1::ecdsa::RecoverableSignature::from_compact(&raw_sig, recovery_id)
    else {
        return Ok(Bytes::new());
    };

    let message = secp256k1::Message::from_digest(raw_hash);

    let Ok(public_key) = recoverable_signature.recover(&message) else {
        return Ok(Bytes::new());
    };

    // We need to take the 64 bytes from the public key (discarding the first pos of the slice)
    let public_key_hash =
        ethrex_crypto::keccak::keccak_hash(&public_key.serialize_uncompressed()[1..]);

    // Address is the last 20 bytes of the hash.
    let recovered_address_bytes = &public_key_hash[12..];

    let mut out = [0u8; 32];

    out[12..32].copy_from_slice(recovered_address_bytes);

    Ok(Bytes::copy_from_slice(&out))
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
#[cfg(any(
    feature = "sp1",
    feature = "risc0",
    feature = "zisk",
    not(feature = "secp256k1"),
))]
pub fn ecrecover(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    use ethrex_common::utils::keccak;
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    use crate::gas_cost::ECRECOVER_COST;

    increase_precompile_consumed_gas(ECRECOVER_COST, gas_remaining)?;

    const INPUT_LEN: usize = 128;
    const WORD: usize = 32;

    let input = fill_with_zeros(calldata, INPUT_LEN);

    let (raw_hash, tail) = input.split_at(WORD);
    let (raw_v, raw_sig) = tail.split_at(WORD);

    // EVM expects v ∈ {27, 28}. Anything else is invalid → empty return.
    let mut recid_byte = match u8::try_from(u256_from_big_endian(raw_v)) {
        Ok(27) => 0,
        Ok(28) => 1,
        _ => return Ok(Bytes::new()),
    };

    // Parse signature (r||s). If malformed → empty return.
    let Ok(mut sig) = Signature::from_slice(raw_sig) else {
        return Ok(Bytes::new());
    };

    // k256 enforces canonical low-S for recovery.
    // If S is high, normalize s := n - s and flip the recovery parity bit.
    if let Some(low_s) = sig.normalize_s() {
        sig = low_s;
        recid_byte ^= 1;
    }

    // Recovery id from the adjusted byte.
    let Some(recid) = RecoveryId::from_byte(recid_byte) else {
        return Ok(Bytes::new());
    };

    // Recover the verifying key from the prehash (32-byte digest).
    let Ok(vk) = VerifyingKey::recover_from_prehash(raw_hash, &sig, recid) else {
        return Ok(Bytes::new());
    };

    // SEC1 uncompressed: 0x04 || X(32) || Y(32). We need X||Y (64 bytes).
    let uncompressed = vk.to_encoded_point(false);
    let mut uncompressed = uncompressed.to_bytes();
    #[allow(clippy::indexing_slicing)]
    let xy = &mut uncompressed[1..65];

    // keccak256(X||Y).
    let xy = keccak(xy);

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
    Ok(Bytes::copy_from_slice(&digest))
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
    if let Some(data) = calldata.get(lower_limit..upper_limit)
        && !data.is_empty()
    {
        if data.len() == size_to_expand {
            return data.into();
        }
        let mut extended = vec![0u8; size_to_expand];
        let copy_size = size_to_expand.min(data.len());
        extended[..copy_size].copy_from_slice(&data[..copy_size]);
        return extended.into();
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
        #[cfg(not(feature = "zisk"))]
        {
            let base_mod = base % &modulus; // malachite requires base to be reduced to modulus first
            base_mod.mod_pow(&exponent, &modulus)
        }

        #[cfg(feature = "zisk")]
        {
            use ziskos::zisklib::modexp_u64;
            let (mut base, mut exponent, mut modulus) = (base, exponent, modulus);
            let base_limbs = base.to_limbs_asc();
            let exponent_limbs = exponent.to_limbs_asc();
            let modulus_limbs = modulus.to_limbs_asc();
            let result_limbs = modexp_u64(&base_limbs, &exponent_limbs, &modulus_limbs);
            Natural::from_owned_limbs_asc(result_limbs)
        }
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

    let (Some(first_point), Some(second_point)) =
        (parse_bn254_g1(&calldata, 0), parse_bn254_g1(&calldata, 64))
    else {
        return Err(InternalError::Slicing.into());
    };
    validate_bn254_g1_coords(&first_point)?;
    validate_bn254_g1_coords(&second_point)?;
    bn254_g1_add(first_point, second_point)
}

#[cfg(not(feature = "zisk"))]
#[inline]
pub fn bn254_g1_add(first_point: G1, second_point: G1) -> Result<Bytes, VMError> {
    let first_point_x = ark_bn254::Fq::from_be_bytes_mod_order(&first_point.0.to_big_endian());
    let first_point_y = ark_bn254::Fq::from_be_bytes_mod_order(&first_point.1.to_big_endian());
    let second_point_x = ark_bn254::Fq::from_be_bytes_mod_order(&second_point.0.to_big_endian());
    let second_point_y = ark_bn254::Fq::from_be_bytes_mod_order(&second_point.1.to_big_endian());

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

#[cfg(feature = "zisk")]
#[inline]
pub fn bn254_g1_add(first_point: G1, second_point: G1) -> Result<Bytes, VMError> {
    // SP1 patches the substrate-bn crate too, but some Ethereum Mainnet blocks fail to execute with it with a GasMismatch error
    // so for now we will only use it for ZisK.
    use substrate_bn::{AffineG1, Fq, G1 as SubstrateG1, Group};

    if first_point.is_zero() && second_point.is_zero() {
        return Ok(Bytes::from([0u8; 64].to_vec()));
    }

    if first_point.is_zero() {
        let (g1_x, g1_y) = (
            Fq::from_slice(&second_point.0.to_big_endian())
                .map_err(|_| PrecompileError::ParsingInputError)?,
            Fq::from_slice(&second_point.1.to_big_endian())
                .map_err(|_| PrecompileError::ParsingInputError)?,
        );
        // Validate that the point is on the curve
        AffineG1::new(g1_x, g1_y).map_err(|_| PrecompileError::InvalidPoint)?;

        let mut x_bytes = [0u8; 32];
        let mut y_bytes = [0u8; 32];
        g1_x.to_big_endian(&mut x_bytes);
        g1_y.to_big_endian(&mut y_bytes);
        return Ok(Bytes::from([x_bytes, y_bytes].concat()));
    }

    if second_point.is_zero() {
        let (g1_x, g1_y) = (
            Fq::from_slice(&first_point.0.to_big_endian())
                .map_err(|_| PrecompileError::ParsingInputError)?,
            Fq::from_slice(&first_point.1.to_big_endian())
                .map_err(|_| PrecompileError::ParsingInputError)?,
        );
        // Validate that the point is on the curve
        AffineG1::new(g1_x, g1_y).map_err(|_| PrecompileError::InvalidPoint)?;

        let mut x_bytes = [0u8; 32];
        let mut y_bytes = [0u8; 32];
        g1_x.to_big_endian(&mut x_bytes);
        g1_y.to_big_endian(&mut y_bytes);
        return Ok(Bytes::from([x_bytes, y_bytes].concat()));
    }

    let (first_x, first_y) = (
        Fq::from_slice(&first_point.0.to_big_endian())
            .map_err(|_| PrecompileError::ParsingInputError)?,
        Fq::from_slice(&first_point.1.to_big_endian())
            .map_err(|_| PrecompileError::ParsingInputError)?,
    );

    let (second_x, second_y) = (
        Fq::from_slice(&second_point.0.to_big_endian())
            .map_err(|_| PrecompileError::ParsingInputError)?,
        Fq::from_slice(&second_point.1.to_big_endian())
            .map_err(|_| PrecompileError::ParsingInputError)?,
    );

    let first: SubstrateG1 = AffineG1::new(first_x, first_y)
        .map_err(|_| PrecompileError::InvalidPoint)?
        .into();

    let second: SubstrateG1 = AffineG1::new(second_x, second_y)
        .map_err(|_| PrecompileError::InvalidPoint)?
        .into();

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "G1 addition doesn't overflow, intermediate operations that could overflow should be handled correctly by the library"
    )]
    let result = first + second;

    let mut x_bytes = [0u8; 32];
    let mut y_bytes = [0u8; 32];
    result.x().to_big_endian(&mut x_bytes);
    result.y().to_big_endian(&mut y_bytes);
    let out = [x_bytes, y_bytes].concat();

    Ok(Bytes::from(out))
}

/// Makes a scalar multiplication on the elliptic curve 'alt_bn128'
pub fn ecmul(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 96);
    increase_precompile_consumed_gas(ECMUL_COST, gas_remaining)?;

    let (Some(g1), Some(scalar)) = (
        parse_bn254_g1(&calldata, 0),
        parse_bn254_scalar(&calldata, 64),
    ) else {
        return Err(InternalError::Slicing.into());
    };
    validate_bn254_g1_coords(&g1)?;
    bn254_g1_mul(g1, scalar)
}

#[cfg(not(any(feature = "sp1", feature = "zisk")))]
#[inline]
pub fn bn254_g1_mul(point: G1, scalar: U256) -> Result<Bytes, VMError> {
    let x = ark_bn254::Fq::from_be_bytes_mod_order(&point.0.to_big_endian());
    let y = ark_bn254::Fq::from_be_bytes_mod_order(&point.1.to_big_endian());

    if x.is_zero() && y.is_zero() {
        return Ok(Bytes::from([0u8; 64].to_vec()));
    }

    let point = G1AffineArk::new_unchecked(x, y);
    if !point.is_on_curve() {
        return Err(PrecompileError::InvalidPoint.into());
    }

    let scalar = FrArk::from_be_bytes_mod_order(&scalar.to_big_endian());
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

#[cfg(any(feature = "sp1", feature = "zisk"))]
#[inline]
pub fn bn254_g1_mul(g1: G1, scalar: U256) -> Result<Bytes, VMError> {
    use substrate_bn::{AffineG1, Fq, Fr, G1, Group};

    if g1.is_zero() || scalar.is_zero() {
        return Ok(Bytes::from([0u8; 64].to_vec()));
    }

    let (g1_x, g1_y) = (
        Fq::from_slice(&g1.0.to_big_endian()).map_err(|_| PrecompileError::ParsingInputError)?,
        Fq::from_slice(&g1.1.to_big_endian()).map_err(|_| PrecompileError::ParsingInputError)?,
    );

    let g1 = AffineG1::new(g1_x, g1_y).map_err(|_| PrecompileError::InvalidPoint)?;

    // Small difference between the patched versions of substrate-bn
    #[cfg(feature = "zisk")]
    let g1: G1 = g1.into();

    let scalar =
        Fr::from_slice(&scalar.to_big_endian()).map_err(|_| PrecompileError::ParsingInputError)?;

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "G1 scalar multiplication doesn't overflow, intermediate operations that could overflow should be handled correctly by the library"
    )]
    let result = g1 * scalar;

    let mut x_bytes = [0u8; 32];
    let mut y_bytes = [0u8; 32];
    result.x().to_big_endian(&mut x_bytes);
    result.y().to_big_endian(&mut y_bytes);
    let out = [x_bytes, y_bytes].concat();

    Ok(Bytes::from(out))
}

const ALT_BN128_PRIME: U256 = U256([
    0x3c208c16d87cfd47,
    0x97816a916871ca8d,
    0xb85045b68181585d,
    0x30644e72e131a029,
]);

pub struct G1(U256, U256);
impl G1 {
    /// According to EIP-197, the point at infinity (also called neutral element of G1 or zero) is encoded as (0, 0)
    pub fn is_zero(&self) -> bool {
        self.0.is_zero() && self.1.is_zero()
    }
}
pub struct G2(U256, U256, U256, U256);
impl G2 {
    /// According to EIP-197, the point at infinity (also called neutral element of G2 or zero) is encoded as (0, 0, 0, 0)
    pub fn is_zero(&self) -> bool {
        self.0.is_zero() && self.1.is_zero() && self.2.is_zero() && self.3.is_zero()
    }
}

/// Parses 32 bytes as BN254 scalar
#[inline]
fn parse_bn254_scalar(buf: &[u8], offset: usize) -> Option<U256> {
    buf.get(offset..offset.checked_add(32)?)
        .map(u256_from_big_endian)
}

/// Parses 64 bytes as a BN254 G1 point
#[inline]
fn parse_bn254_g1(buf: &[u8], offset: usize) -> Option<G1> {
    let chunk = buf.get(offset..offset.checked_add(64)?)?;
    let (x_bytes, y_bytes) = chunk.split_at_checked(32)?;
    Some(G1(
        u256_from_big_endian(x_bytes),
        u256_from_big_endian(y_bytes),
    ))
}

/// Parses 128 bytes as a BN254 G2 point
fn parse_bn254_g2(buf: &[u8], offset: usize) -> Option<G2> {
    let chunk = buf.get(offset..offset.checked_add(128)?)?;
    let (g2_xy, rest) = chunk.split_at_checked(32)?;
    let (g2_xx, rest) = rest.split_at_checked(32)?;
    let (g2_yy, g2_yx) = rest.split_at_checked(32)?;
    Some(G2(
        u256_from_big_endian(g2_xx),
        u256_from_big_endian(g2_xy),
        u256_from_big_endian(g2_yx),
        u256_from_big_endian(g2_yy),
    ))
}

#[inline]
fn validate_bn254_g1_coords(g1: &G1) -> Result<(), VMError> {
    // check each element is in field
    if g1.0 >= ALT_BN128_PRIME || g1.1 >= ALT_BN128_PRIME {
        return Err(PrecompileError::CoordinateExceedsFieldModulus.into());
    }
    Ok(())
}

#[inline]
fn validate_bn254_g2_coords(g2: &G2) -> Result<(), VMError> {
    // check each element is in field
    if g2.0 >= ALT_BN128_PRIME
        || g2.1 >= ALT_BN128_PRIME
        || g2.2 >= ALT_BN128_PRIME
        || g2.3 >= ALT_BN128_PRIME
    {
        return Err(PrecompileError::CoordinateExceedsFieldModulus.into());
    }
    Ok(())
}

/// Performs a bilinear pairing on points on the elliptic curve 'alt_bn128', returns 1 on success and 0 on failure
pub fn ecpairing(calldata: &Bytes, gas_remaining: &mut u64, _fork: Fork) -> Result<Bytes, VMError> {
    // The input must always be a multiple of 192 (6 32-byte values)
    if !calldata.len().is_multiple_of(192) {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let inputs_amount = calldata.len() / 192;
    let gas_cost = gas_cost::ecpairing(inputs_amount)?;
    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let mut batch = Vec::new();
    for input in calldata.chunks_exact(192) {
        let (Some(g1), Some(g2)) = (parse_bn254_g1(input, 0), parse_bn254_g2(input, 64)) else {
            return Err(InternalError::Slicing.into());
        };
        validate_bn254_g1_coords(&g1)?;
        validate_bn254_g2_coords(&g2)?;
        batch.push((g1, g2));
    }

    let pairing_check = if batch.is_empty() {
        true
    } else {
        pairing_check(&batch)?
    };

    let mut result = [0; 32];
    result[31] = u8::from(pairing_check);
    Ok(Bytes::from_owner(result))
}

#[cfg(any(feature = "sp1", feature = "risc0", feature = "zisk"))]
#[inline]
pub fn pairing_check(batch: &[(G1, G2)]) -> Result<bool, VMError> {
    use substrate_bn::{AffineG1, AffineG2, Fq, Fq2, G1 as SubstrateG1, G2 as SubstrateG2, Group};

    if batch.is_empty() {
        return Ok(true);
    }
    let mut valid_batch = Vec::with_capacity(batch.len());
    for (g1, g2) in batch {
        let g1: SubstrateG1 = if g1.is_zero() {
            SubstrateG1::zero()
        } else {
            let (g1_x, g1_y) = (
                Fq::from_slice(&g1.0.to_big_endian())
                    .map_err(|_| PrecompileError::ParsingInputError)?,
                Fq::from_slice(&g1.1.to_big_endian())
                    .map_err(|_| PrecompileError::ParsingInputError)?,
            );
            AffineG1::new(g1_x, g1_y)
                .map_err(|_| PrecompileError::InvalidPoint)?
                .into()
        };
        let g2: SubstrateG2 = if g2.is_zero() {
            SubstrateG2::zero()
        } else {
            let (g2_x, g2_y) = (
                Fq2::new(
                    Fq::from_slice(&g2.0.to_big_endian())
                        .map_err(|_| PrecompileError::ParsingInputError)?,
                    Fq::from_slice(&g2.1.to_big_endian())
                        .map_err(|_| PrecompileError::ParsingInputError)?,
                ),
                Fq2::new(
                    Fq::from_slice(&g2.2.to_big_endian())
                        .map_err(|_| PrecompileError::ParsingInputError)?,
                    Fq::from_slice(&g2.3.to_big_endian())
                        .map_err(|_| PrecompileError::ParsingInputError)?,
                ),
            );
            AffineG2::new(g2_x, g2_y)
                .map_err(|_| PrecompileError::InvalidPoint)?
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

#[cfg(all(not(feature = "sp1"), not(feature = "risc0"), not(feature = "zisk")))]
#[inline]
pub fn pairing_check(batch: &[(G1, G2)]) -> Result<bool, VMError> {
    use ark_bn254::{Bn254, G1Affine, G2Affine};
    use ark_ec::pairing::Pairing;
    use ark_ff::{Fp, One, QuadExtField};

    let mut g1_points = Vec::with_capacity(batch.len());
    let mut g2_points = Vec::with_capacity(batch.len());

    for (g1, g2) in batch {
        g1_points.push(if g1.is_zero() {
            G1Affine::identity()
        } else {
            let p = G1Affine::new_unchecked(
                Fp::from_le_bytes_mod_order(&g1.0.to_little_endian()),
                Fp::from_le_bytes_mod_order(&g1.1.to_little_endian()),
            );

            if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
                return Err(PrecompileError::InvalidPoint.into());
            }

            p
        });

        g2_points.push(if g2.is_zero() {
            G2Affine::identity()
        } else {
            let p = G2Affine::new_unchecked(
                QuadExtField::new(
                    Fp::from_le_bytes_mod_order(&g2.0.to_little_endian()),
                    Fp::from_le_bytes_mod_order(&g2.1.to_little_endian()),
                ),
                QuadExtField::new(
                    Fp::from_le_bytes_mod_order(&g2.2.to_little_endian()),
                    Fp::from_le_bytes_mod_order(&g2.3.to_little_endian()),
                ),
            );

            if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
                return Err(PrecompileError::InvalidPoint.into());
            }

            p
        });
    }

    Ok(Bn254::multi_pairing(g1_points, g2_points).0 == QuadExtField::one())
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

pub fn bls12_g1add(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    // TODO: Use `as_chunks` after upgrading to Rust 1.88.0.
    let (x_data, calldata) = calldata
        .split_first_chunk::<128>()
        .ok_or(PrecompileError::ParsingInputError)?;
    let (y_data, calldata) = calldata
        .split_first_chunk::<128>()
        .ok_or(PrecompileError::ParsingInputError)?;
    if !calldata.is_empty() {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // Apply precompile gas cost.
    increase_precompile_consumed_gas(BLS12_381_G1ADD_COST, gas_remaining)
        .map_err(|_| PrecompileError::NotEnoughGas)?;

    type FElem = BLS12381FieldElement;
    type U384 = UnsignedInteger<6>;
    fn parse_g1_point(data: &[u8; 128]) -> Result<Option<(FElem, FElem)>, PrecompileError> {
        if data[0..16] != [0; 16] || data[64..80] != [0; 16] {
            return Err(PrecompileError::ParsingInputError);
        }

        let x = U384::from_bytes_be(&data[16..64]).unwrap_or_default();
        let y = U384::from_bytes_be(&data[80..128]).unwrap_or_default();
        if x >= BLS12381FieldModulus::MODULUS || y >= BLS12381FieldModulus::MODULUS {
            return Err(PrecompileError::ParsingInputError);
        }

        if x == U384::from_u64(0) && y == U384::from_u64(0) {
            return Ok(None);
        }

        let x = FElem::new(x);
        let y = FElem::new(y);
        if BLS12381Curve::defining_equation(&x, &y) != FElem::zero() {
            return Err(PrecompileError::BLS12381G1PointNotInCurve);
        }

        Ok(Some((x, y)))
    }

    let p0 = parse_g1_point(x_data)?;
    let p1 = parse_g1_point(y_data)?;

    #[expect(clippy::arithmetic_side_effects, reason = "modular arithmetic")]
    let p2 = match (p0, p1) {
        (None, None) => (FElem::zero(), FElem::zero()),
        (None, Some(p1)) => p1,
        (Some(p0), None) => p0,
        (Some(p0), Some(p1)) => 'block: {
            if p0.0 == p1.0 {
                if p0.1 == p1.1 {
                    // The division may panic only when `p0.1.double()` has no inverse. This can
                    // only happen if `p0.1 == 0`, which is impossible as long as the defining
                    // equation holds since it has no solutions for an `x` coordinate where `y` is
                    // zero within the prime field space.
                    let x_squared = p0.0.square();
                    let s = ((x_squared.double() + &x_squared + BLS12381Curve::a())
                        / p0.1.double())
                    .map_err(|_e| VMError::Internal(InternalError::DivisionByZero))?;

                    let x = s.square() - p0.0.double();
                    let y = s * (p0.0 - &x) - p0.1;
                    break 'block (x, y);
                } else if &p0.1 + &p1.1 == FElem::zero() {
                    break 'block (FElem::zero(), FElem::zero());
                }
            }

            // The division may panic only when `t` has no inverse. This can only happen if
            // `p0.0 == p1.0`, for which the defining equation gives us two possible values for
            // `p0.1` and `p1.1`, which are 2 and -2. Both cases have already been handled before.
            let l = ((&p0.1 - p1.1) / (&p0.0 - &p1.0))
                .map_err(|_e| VMError::Internal(InternalError::DivisionByZero))?;

            let x = l.square() - &p0.0 - p1.0;
            let y = l * (p0.0 - &x) - p0.1;
            (x, y)
        }
    };

    let x = p2.0.representative().limbs.map(|x| x.to_be_bytes());
    let y = p2.1.representative().limbs.map(|x| x.to_be_bytes());
    let buffer: [[u8; 8]; 16] = [
        [0; 8], [0; 8], x[0], x[1], x[2], x[3], x[4], x[5], // Padded x coordinate.
        [0; 8], [0; 8], y[0], y[1], y[2], y[3], y[4], y[5], // Padded y coordinate.
    ];
    Ok(Bytes::copy_from_slice(buffer.as_flattened()))
}

pub fn bls12_g1msm(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.is_empty() || !calldata.len().is_multiple_of(BLS12_381_G1_MSM_PAIR_LENGTH) {
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

pub fn bls12_g2add(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    // TODO: Use `as_chunks` after upgrading to Rust 1.88.0.
    let (x_data, calldata) = calldata
        .split_first_chunk::<256>()
        .ok_or(PrecompileError::ParsingInputError)?;
    let (y_data, calldata) = calldata
        .split_first_chunk::<256>()
        .ok_or(PrecompileError::ParsingInputError)?;
    if !calldata.is_empty() {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // Apply precompile gas cost.
    increase_precompile_consumed_gas(BLS12_381_G2ADD_COST, gas_remaining)
        .map_err(|_| PrecompileError::NotEnoughGas)?;

    type FElem = BLS12381TwistCurveFieldElement;
    type U384 = UnsignedInteger<6>;
    fn parse_g2_point(data: &[u8; 256]) -> Result<Option<(FElem, FElem)>, PrecompileError> {
        if data[0..16] != [0; 16]
            || data[64..80] != [0; 16]
            || data[128..144] != [0; 16]
            || data[192..208] != [0; 16]
        {
            return Err(PrecompileError::ParsingInputError);
        }

        let x = [
            U384::from_bytes_be(&data[16..64]).unwrap_or_default(),
            U384::from_bytes_be(&data[80..128]).unwrap_or_default(),
        ];
        let y = [
            U384::from_bytes_be(&data[144..192]).unwrap_or_default(),
            U384::from_bytes_be(&data[208..256]).unwrap_or_default(),
        ];
        if x[0] >= BLS12381FieldModulus::MODULUS
            || x[1] >= BLS12381FieldModulus::MODULUS
            || y[0] >= BLS12381FieldModulus::MODULUS
            || y[1] >= BLS12381FieldModulus::MODULUS
        {
            return Err(PrecompileError::ParsingInputError);
        }

        if x[0] == U384::from_u64(0)
            && x[1] == U384::from_u64(0)
            && y[0] == U384::from_u64(0)
            && y[1] == U384::from_u64(0)
        {
            return Ok(None);
        }

        let x = FElem::from_raw(x.map(BLS12381FieldElement::new));
        let y = FElem::from_raw(y.map(BLS12381FieldElement::new));
        if BLS12381TwistCurve::defining_equation(&x, &y) != FElem::zero() {
            return Err(PrecompileError::BLS12381G2PointNotInCurve);
        }

        Ok(Some((x, y)))
    }

    let p0 = parse_g2_point(x_data)?;
    let p1 = parse_g2_point(y_data)?;

    #[expect(clippy::arithmetic_side_effects, reason = "modular arithmetic")]
    let p2 = match (p0, p1) {
        (None, None) => (FElem::zero(), FElem::zero()),
        (None, Some(p1)) => p1,
        (Some(p0), None) => p0,
        (Some(p0), Some(p1)) => 'block: {
            if p0.0 == p1.0 {
                if p0.1 == p1.1 {
                    // The division may panic only when `p0.1.double()` has no inverse. This can
                    // only happen if `p0.1 == 0`, which is impossible as long as the defining
                    // equation holds since it has no solutions for an `x` coordinate where `y` is
                    // zero within the prime field space.
                    let x_squared = p0.0.square();
                    let s = ((x_squared.double() + &x_squared + BLS12381TwistCurve::a())
                        / p0.1.double())
                    .map_err(|_e| VMError::Internal(InternalError::DivisionByZero))?;

                    let x = s.square() - p0.0.double();
                    let y = s * (p0.0 - &x) - p0.1;
                    break 'block (x, y);
                } else if &p0.1 + &p1.1 == FElem::zero() {
                    break 'block (FElem::zero(), FElem::zero());
                }
            }

            // The division may panic only when `t` has no inverse. This can only happen if
            // `p0.0 == p1.0`, for which the defining equation gives us two possible values for
            // `p0.1` and `p1.1`, which are 2 and -2. Both cases have already been handled before.
            let l = ((&p0.1 - p1.1) / (&p0.0 - &p1.0))
                .map_err(|_e| VMError::Internal(InternalError::DivisionByZero))?;

            let x = l.square() - &p0.0 - p1.0;
            let y = l * (p0.0 - &x) - p0.1;
            (x, y)
        }
    };

    let p2 = (p2.0.to_raw(), p2.1.to_raw());
    let x = (
        p2.0[0].representative().limbs.map(|x| x.to_be_bytes()),
        p2.0[1].representative().limbs.map(|x| x.to_be_bytes()),
    );
    let y = (
        p2.1[0].representative().limbs.map(|x| x.to_be_bytes()),
        p2.1[1].representative().limbs.map(|x| x.to_be_bytes()),
    );
    let buffer: [[u8; 8]; 32] = [
        [0; 8], [0; 8], x.0[0], x.0[1], x.0[2], x.0[3], x.0[4], x.0[5], //
        [0; 8], [0; 8], x.1[0], x.1[1], x.1[2], x.1[3], x.1[4], x.1[5], //
        [0; 8], [0; 8], y.0[0], y.0[1], y.0[2], y.0[3], y.0[4], y.0[5], //
        [0; 8], [0; 8], y.1[0], y.1[1], y.1[2], y.1[3], y.1[4], y.1[5], //
    ];
    Ok(Bytes::copy_from_slice(buffer.as_flattened()))
}

pub fn bls12_g2msm(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    if calldata.is_empty() || !calldata.len().is_multiple_of(BLS12_381_G2_MSM_PAIR_LENGTH) {
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
    if calldata.is_empty()
        || !calldata
            .len()
            .is_multiple_of(BLS12_381_PAIRING_CHECK_PAIR_LENGTH)
    {
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
