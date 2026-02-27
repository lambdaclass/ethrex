use bytes::{Buf, Bytes};
use ethrex_common::H160;
use ethrex_common::utils::u256_from_big_endian_const;
use ethrex_common::{
    Address, H256, U256, types::Fork, types::Fork::*, utils::u256_from_big_endian,
};
use ethrex_crypto::{CryptoError, Crypto};
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::sync::RwLock;

use crate::gas_cost::{MODEXP_STATIC_COST, P256_VERIFY_COST};
use crate::vm::VMType;
use crate::{
    constants::{VERSIONED_HASH_VERSION_KZG},
    errors::{InternalError, PrecompileError, VMError},
    gas_cost::{
        self, BLAKE2F_ROUND_COST, BLS12_381_G1_K_DISCOUNT, BLS12_381_G1ADD_COST,
        BLS12_381_G2_K_DISCOUNT, BLS12_381_G2ADD_COST, BLS12_381_MAP_FP_TO_G1_COST,
        BLS12_381_MAP_FP2_TO_G2_COST, ECADD_COST, ECMUL_COST, G1_MUL_COST, G2_MUL_COST,
        POINT_EVALUATION_COST,
    },
};

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

pub const PRECOMPILES: [Precompile; 18] = [
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
    BLS12_PAIRING_CHECK,
    BLS12_MAP_FP_TO_G1,
    BLS12_MAP_FP2_TO_G2,
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
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    type PrecompileFn = fn(&Bytes, &mut u64, Fork, &dyn Crypto) -> Result<Bytes, VMError>;

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
            Some(bls12_map_fp2_to_g2 as PrecompileFn);
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
    let result = precompile(calldata, gas_remaining, fork, crypto);

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

fn crypto_error_to_precompile(e: CryptoError) -> VMError {
    match e {
        CryptoError::InvalidPoint(_) => PrecompileError::InvalidPoint.into(),
        CryptoError::InvalidInput(_) => PrecompileError::ParsingInputError.into(),
        CryptoError::VerificationFailed => PrecompileError::ParsingInputError.into(),
        _ => PrecompileError::ParsingInputError.into(),
    }
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
pub fn ecrecover(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    use crate::gas_cost::ECRECOVER_COST;

    increase_precompile_consumed_gas(ECRECOVER_COST, gas_remaining)?;

    const INPUT_LEN: usize = 128;
    const WORD: usize = 32;

    let input = fill_with_zeros(calldata, INPUT_LEN);

    #[expect(clippy::indexing_slicing, reason = "fill_with_zeros guarantees len >= 128")]
    let raw_hash: &[u8] = &input[0..WORD];
    #[expect(clippy::indexing_slicing, reason = "fill_with_zeros guarantees len >= 128")]
    let raw_v: &[u8] = &input[WORD..WORD * 2];
    #[expect(clippy::indexing_slicing, reason = "fill_with_zeros guarantees len >= 128")]
    let raw_sig: &[u8] = &input[WORD * 2..WORD * 2 + 64];

    // EVM expects v ∈ {27, 28}. Anything else is invalid → empty return.
    let recid_byte: u8 = match u8::try_from(u256_from_big_endian(raw_v)) {
        Ok(27) => 0,
        Ok(28) => 1,
        _ => return Ok(Bytes::new()),
    };

    let msg_hash: [u8; 32] = raw_hash
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;
    let sig: [u8; 64] = raw_sig
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    let pk_hash = match crypto.secp256k1_ecrecover(&sig, recid_byte, &msg_hash) {
        Ok(h) => h,
        Err(_) => return Ok(Bytes::new()),
    };

    // Address is the last 20 bytes of the keccak hash of the public key.
    let mut out = [0u8; 32];
    #[expect(clippy::indexing_slicing, reason = "pk_hash is 32 bytes, out is 32 bytes")]
    out[12..32].copy_from_slice(&pk_hash[12..32]);

    Ok(Bytes::copy_from_slice(&out))
}

/// Returns the calldata received
pub fn identity(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    _crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::identity(calldata.len())?;

    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    Ok(calldata.clone())
}

/// Returns the calldata hashed by sha2-256 algorithm
pub fn sha2_256(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::sha2_256(calldata.len())?;

    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let digest = crypto.sha256(calldata);
    Ok(Bytes::copy_from_slice(&digest))
}

/// Returns the calldata hashed by ripemd-160 algorithm, padded by zeros at left
pub fn ripemd_160(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    let gas_cost = gas_cost::ripemd_160(calldata.len())?;

    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let result = crypto.ripemd160(calldata);
    Ok(Bytes::copy_from_slice(&result))
}

/// Returns the result of the module-exponentiation operation
#[expect(clippy::indexing_slicing, reason = "bounds checked at start")]
pub fn modexp(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
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

    // Gas computation uses malachite Natural to compute bit length of exponent
    use malachite::Natural;
    use malachite::base::num::conversion::traits::*;
    let exp_first_32_bytes = e.get(0..32.min(exponent_size)).unwrap_or_default();
    let exp_first_32 = Natural::from_power_of_2_digits_desc(8u64, exp_first_32_bytes.iter().cloned())
        .ok_or(InternalError::TypeConversion)?;

    let gas_cost = gas_cost::modexp(&exp_first_32, base_size, exponent_size, modulus_size, fork)?;

    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    if base_size == 0 && modulus_size == 0 {
        return Ok(Bytes::new());
    }

    let result = crypto
        .modexp(&b, &e, &m)
        .map_err(|_| VMError::from(PrecompileError::ParsingInputError))?;

    let res_bytes = increase_left_pad(&Bytes::from(result), modulus_size);

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
pub fn ecadd(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
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

    #[expect(clippy::indexing_slicing, reason = "calldata padded to 128 bytes")]
    let result = crypto
        .bn254_g1_add(&calldata[..64], &calldata[64..128])
        .map_err(crypto_error_to_precompile)?;

    Ok(Bytes::copy_from_slice(&result))
}

/// Makes a scalar multiplication on the elliptic curve 'alt_bn128'
pub fn ecmul(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    // If calldata does not reach the required length, we should fill the rest with zeros
    let calldata = fill_with_zeros(calldata, 96);
    increase_precompile_consumed_gas(ECMUL_COST, gas_remaining)?;

    let (Some(g1), Some(_scalar)) = (
        parse_bn254_g1(&calldata, 0),
        parse_bn254_scalar(&calldata, 64),
    ) else {
        return Err(InternalError::Slicing.into());
    };
    validate_bn254_g1_coords(&g1)?;

    #[expect(clippy::indexing_slicing, reason = "calldata padded to 96 bytes")]
    let result = crypto
        .bn254_g1_mul(&calldata[..64], &calldata[64..96])
        .map_err(crypto_error_to_precompile)?;

    Ok(Bytes::copy_from_slice(&result))
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
pub fn ecpairing(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    // The input must always be a multiple of 192 (6 32-byte values)
    if !calldata.len().is_multiple_of(192) {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let inputs_amount = calldata.len() / 192;
    let gas_cost = gas_cost::ecpairing(inputs_amount)?;
    increase_precompile_consumed_gas(gas_cost, gas_remaining)?;

    let mut pairs: Vec<(&[u8], &[u8])> = Vec::new();
    for input in calldata.chunks_exact(192) {
        let (Some(g1), Some(g2)) = (parse_bn254_g1(input, 0), parse_bn254_g2(input, 64)) else {
            return Err(InternalError::Slicing.into());
        };
        validate_bn254_g1_coords(&g1)?;
        validate_bn254_g2_coords(&g2)?;
        #[expect(clippy::indexing_slicing, reason = "chunks_exact guarantees 192 bytes")]
        pairs.push((&input[..64], &input[64..192]));
    }

    let pairing_check = if pairs.is_empty() {
        true
    } else {
        crypto
            .bn254_pairing_check(&pairs)
            .map_err(crypto_error_to_precompile)?
    };

    let mut result = [0; 32];
    result[31] = u8::from(pairing_check);
    Ok(Bytes::from_owner(result))
}

/// Returns the result of Blake2 hashing algorithm given a certain parameters from the calldata.
pub fn blake2f(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
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

    crypto.blake2_compress(rounds, &mut h, m, t, f);

    Ok(Bytes::from_iter(
        h.into_iter().flat_map(|value| value.to_le_bytes()),
    ))
}

/// Converts the provided commitment to match the provided versioned_hash.
fn kzg_commitment_to_versioned_hash(commitment_bytes: &[u8; 48], crypto: &dyn Crypto) -> H256 {
    let mut versioned_hash: [u8; 32] = crypto.sha256(commitment_bytes);
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
    crypto: &dyn Crypto,
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

    let z: [u8; 32] = calldata
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
    if kzg_commitment_to_versioned_hash(&commitment, crypto) != H256::from(versioned_hash) {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // This verifies the proof from a point (z, y) and a commitment
    crypto
        .verify_kzg_proof(&z, &y, &commitment, &proof)
        .map_err(|_| VMError::from(PrecompileError::ParsingInputError))?;

    // The first 32 bytes consist of the number of field elements in the blob, and the
    // other 32 bytes consist of the modulus used in the BLS signature scheme.
    let output = POINT_EVALUATION_OUTPUT_BYTES.to_vec();

    Ok(Bytes::from(output))
}

/// Signature verification in the "secp256r1" elliptic curve
/// If the verification succeeds, returns 1 in a 32-bit big-endian format.
/// If the verification fails, returns an empty `Bytes` object.
/// Implemented following https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7951.md
pub fn p_256_verify(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
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
    let msg: &[u8; 32] = calldata[0..32].try_into()?;
    #[expect(clippy::indexing_slicing, reason = "length checked")]
    let r: &[u8; 32] = calldata[32..64].try_into()?;
    #[expect(clippy::indexing_slicing, reason = "length checked")]
    let s: &[u8; 32] = calldata[64..96].try_into()?;
    #[expect(clippy::indexing_slicing, reason = "length checked")]
    let pk_x: &[u8; 32] = calldata[96..128].try_into()?;
    #[expect(clippy::indexing_slicing, reason = "length checked")]
    let pk_y: &[u8; 32] = calldata[128..160].try_into()?;

    // Build 64-byte sig (r||s) and 64-byte pk (x||y)
    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r);
    sig_bytes[32..].copy_from_slice(s);

    let mut pk_bytes = [0u8; 64];
    pk_bytes[..32].copy_from_slice(pk_x);
    pk_bytes[32..].copy_from_slice(pk_y);

    let success = crypto.secp256r1_verify(msg, &sig_bytes, &pk_bytes);

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

/// Parse a 64-byte padded BLS12-381 field element into a 48-byte unpadded element.
/// The first 16 bytes must be zero (padding). Returns error if padding is invalid.
fn parse_bls12_padded_fp(
    padded: &[u8; 64],
) -> Result<[u8; 48], VMError> {
    if padded[..16] != [0u8; 16] {
        return Err(PrecompileError::ParsingInputError.into());
    }
    #[expect(clippy::indexing_slicing, reason = "bounds known from array size")]
    let fp: [u8; 48] = padded[16..64]
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;
    Ok(fp)
}


pub fn bls12_g1add(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
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

    // Parse two 128-byte padded G1 points into 48-byte unpadded coordinates.
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let ax = parse_bls12_padded_fp(x_data[..64].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let ay = parse_bls12_padded_fp(x_data[64..128].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let bx = parse_bls12_padded_fp(y_data[..64].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let by = parse_bls12_padded_fp(y_data[64..128].try_into().map_err(|_| InternalError::TypeConversion)?)?;

    let result = crypto
        .bls12_381_g1_add((ax, ay), (bx, by))
        .map_err(crypto_error_to_precompile)?;

    // Re-pad the 96-byte unpadded result (x||y each 48 bytes) to 128 bytes.
    let mut output = [0u8; 128];
    #[expect(clippy::indexing_slicing, reason = "result is 96 bytes")]
    {
        output[16..64].copy_from_slice(&result[..48]);
        output[80..128].copy_from_slice(&result[48..96]);
    }
    Ok(Bytes::copy_from_slice(&output))
}

pub fn bls12_g1msm(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    if calldata.is_empty() || !calldata.len().is_multiple_of(BLS12_381_G1_MSM_PAIR_LENGTH) {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let k = calldata.len() / BLS12_381_G1_MSM_PAIR_LENGTH;
    let required_gas = gas_cost::bls12_msm(k, &BLS12_381_G1_K_DISCOUNT, G1_MUL_COST)?;
    increase_precompile_consumed_gas(required_gas, gas_remaining)?;

    let mut pairs: Vec<(([u8; 48], [u8; 48]), [u8; 32])> = Vec::with_capacity(k);

    #[expect(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        reason = "bounds checked"
    )]
    for i in 0..k {
        let point_offset = i * BLS12_381_G1_MSM_PAIR_LENGTH;
        let scalar_offset = point_offset + 128;
        let pair_end = scalar_offset + 32;

        let point_bytes = &calldata[point_offset..scalar_offset];
        let scalar_bytes = &calldata[scalar_offset..pair_end];

        let px = parse_bls12_padded_fp(point_bytes[..64].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let py = parse_bls12_padded_fp(point_bytes[64..128].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let scalar: [u8; 32] = scalar_bytes.try_into().map_err(|_| InternalError::TypeConversion)?;

        pairs.push(((px, py), scalar));
    }

    let result = crypto
        .bls12_381_g1_msm(&pairs)
        .map_err(crypto_error_to_precompile)?;

    // Re-pad output: 96-byte result → 128-byte padded
    let mut output = [0u8; 128];
    #[expect(clippy::indexing_slicing, reason = "result is 96 bytes")]
    {
        output[16..64].copy_from_slice(&result[..48]);
        output[80..128].copy_from_slice(&result[48..96]);
    }
    Ok(Bytes::copy_from_slice(&output))
}

pub fn bls12_g2add(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
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

    // Parse two 256-byte padded G2 points into four 48-byte unpadded coordinates each.
    // G2 point layout: x_0(64) || x_1(64) || y_0(64) || y_1(64) = 256 bytes
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let ax0 = parse_bls12_padded_fp(x_data[0..64].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let ax1 = parse_bls12_padded_fp(x_data[64..128].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let ay0 = parse_bls12_padded_fp(x_data[128..192].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let ay1 = parse_bls12_padded_fp(x_data[192..256].try_into().map_err(|_| InternalError::TypeConversion)?)?;

    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let bx0 = parse_bls12_padded_fp(y_data[0..64].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let bx1 = parse_bls12_padded_fp(y_data[64..128].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let by0 = parse_bls12_padded_fp(y_data[128..192].try_into().map_err(|_| InternalError::TypeConversion)?)?;
    #[expect(clippy::indexing_slicing, reason = "array sizes known")]
    let by1 = parse_bls12_padded_fp(y_data[192..256].try_into().map_err(|_| InternalError::TypeConversion)?)?;

    let result = crypto
        .bls12_381_g2_add((ax0, ax1, ay0, ay1), (bx0, bx1, by0, by1))
        .map_err(crypto_error_to_precompile)?;

    // Re-pad the 192-byte unpadded result to 256 bytes.
    // Unpadded: x_0(48) || x_1(48) || y_0(48) || y_1(48) = 192 bytes
    // Padded output: x_0_padded(64) || x_1_padded(64) || y_0_padded(64) || y_1_padded(64) = 256 bytes
    let mut output = [0u8; 256];
    #[expect(clippy::indexing_slicing, reason = "result is 192 bytes")]
    {
        output[16..64].copy_from_slice(&result[0..48]);
        output[80..128].copy_from_slice(&result[48..96]);
        output[144..192].copy_from_slice(&result[96..144]);
        output[208..256].copy_from_slice(&result[144..192]);
    }
    Ok(Bytes::copy_from_slice(&output))
}

pub fn bls12_g2msm(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    if calldata.is_empty() || !calldata.len().is_multiple_of(BLS12_381_G2_MSM_PAIR_LENGTH) {
        return Err(PrecompileError::ParsingInputError.into());
    }

    let k = calldata.len() / BLS12_381_G2_MSM_PAIR_LENGTH;
    let required_gas = gas_cost::bls12_msm(k, &BLS12_381_G2_K_DISCOUNT, G2_MUL_COST)?;
    increase_precompile_consumed_gas(required_gas, gas_remaining)?;

    let mut pairs: Vec<(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])> =
        Vec::with_capacity(k);

    #[expect(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        reason = "bounds checked"
    )]
    for i in 0..k {
        let point_offset = i * BLS12_381_G2_MSM_PAIR_LENGTH;
        let scalar_offset = point_offset + 256;
        let pair_end = scalar_offset + 32;

        let point_bytes = &calldata[point_offset..scalar_offset];
        let scalar_bytes = &calldata[scalar_offset..pair_end];

        let x0 = parse_bls12_padded_fp(point_bytes[0..64].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let x1 = parse_bls12_padded_fp(point_bytes[64..128].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let y0 = parse_bls12_padded_fp(point_bytes[128..192].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let y1 = parse_bls12_padded_fp(point_bytes[192..256].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let scalar: [u8; 32] = scalar_bytes.try_into().map_err(|_| InternalError::TypeConversion)?;

        pairs.push(((x0, x1, y0, y1), scalar));
    }

    let result = crypto
        .bls12_381_g2_msm(&pairs)
        .map_err(crypto_error_to_precompile)?;

    // Re-pad the 192-byte unpadded result to 256 bytes.
    let mut output = [0u8; 256];
    #[expect(clippy::indexing_slicing, reason = "result is 192 bytes")]
    {
        output[16..64].copy_from_slice(&result[0..48]);
        output[80..128].copy_from_slice(&result[48..96]);
        output[144..192].copy_from_slice(&result[96..144]);
        output[208..256].copy_from_slice(&result[144..192]);
    }
    Ok(Bytes::copy_from_slice(&output))
}

pub fn bls12_pairing_check(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
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

    let mut pairs: Vec<(([u8; 48], [u8; 48]), ([u8; 48], [u8; 48], [u8; 48], [u8; 48]))> =
        Vec::with_capacity(k);

    #[expect(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        reason = "bounds checked"
    )]
    for i in 0..k {
        let g1_offset = i * BLS12_381_PAIRING_CHECK_PAIR_LENGTH;
        let g2_offset = g1_offset + 128;
        let pair_end = g2_offset + 256;

        let g1_bytes = &calldata[g1_offset..g2_offset];
        let g2_bytes = &calldata[g2_offset..pair_end];

        let g1x = parse_bls12_padded_fp(g1_bytes[0..64].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let g1y = parse_bls12_padded_fp(g1_bytes[64..128].try_into().map_err(|_| InternalError::TypeConversion)?)?;

        let g2x0 = parse_bls12_padded_fp(g2_bytes[0..64].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let g2x1 = parse_bls12_padded_fp(g2_bytes[64..128].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let g2y0 = parse_bls12_padded_fp(g2_bytes[128..192].try_into().map_err(|_| InternalError::TypeConversion)?)?;
        let g2y1 = parse_bls12_padded_fp(g2_bytes[192..256].try_into().map_err(|_| InternalError::TypeConversion)?)?;

        pairs.push(((g1x, g1y), (g2x0, g2x1, g2y0, g2y1)));
    }

    let result = crypto
        .bls12_381_pairing_check(&pairs)
        .map_err(crypto_error_to_precompile)?;

    if result {
        let mut out = vec![0_u8; 31];
        out.push(1);
        Ok(Bytes::from(out))
    } else {
        Ok(Bytes::copy_from_slice(&[0_u8; 32]))
    }
}

pub fn bls12_map_fp_to_g1(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    if calldata.len() != BLS12_381_FP_VALID_INPUT_LENGTH {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // GAS
    increase_precompile_consumed_gas(BLS12_381_MAP_FP_TO_G1_COST, gas_remaining)?;

    // Parse the 64-byte padded field element into 48-byte unpadded.
    #[expect(clippy::indexing_slicing, reason = "bounds checked")]
    let fp = parse_bls12_padded_fp(
        calldata[0..PADDED_FIELD_ELEMENT_SIZE_IN_BYTES]
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?,
    )?;

    let result = crypto
        .bls12_381_fp_to_g1(&fp)
        .map_err(crypto_error_to_precompile)?;

    // Re-pad the 96-byte unpadded G1 result to 128 bytes.
    let mut output = [0u8; 128];
    #[expect(clippy::indexing_slicing, reason = "result is 96 bytes")]
    {
        output[16..64].copy_from_slice(&result[0..48]);
        output[80..128].copy_from_slice(&result[48..96]);
    }
    Ok(Bytes::copy_from_slice(&output))
}

pub fn bls12_map_fp2_to_g2(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    crypto: &dyn Crypto,
) -> Result<Bytes, VMError> {
    if calldata.len() != BLS12_381_FP2_VALID_INPUT_LENGTH {
        return Err(PrecompileError::ParsingInputError.into());
    }

    // GAS
    increase_precompile_consumed_gas(BLS12_381_MAP_FP2_TO_G2_COST, gas_remaining)?;

    // Parse the two 64-byte padded field elements into 48-byte unpadded.
    #[expect(clippy::indexing_slicing, reason = "bounds checked")]
    let c0 = parse_bls12_padded_fp(
        calldata[0..PADDED_FIELD_ELEMENT_SIZE_IN_BYTES]
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?,
    )?;
    #[expect(clippy::indexing_slicing, reason = "bounds checked")]
    let c1 = parse_bls12_padded_fp(
        calldata[PADDED_FIELD_ELEMENT_SIZE_IN_BYTES..BLS12_381_FP2_VALID_INPUT_LENGTH]
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?,
    )?;

    let result = crypto
        .bls12_381_fp2_to_g2((c0, c1))
        .map_err(crypto_error_to_precompile)?;

    // Re-pad the 192-byte unpadded G2 result to 256 bytes.
    // Unpadded: x_0(48) || x_1(48) || y_0(48) || y_1(48) = 192 bytes
    // Padded output: x_0_padded(64) || x_1_padded(64) || y_0_padded(64) || y_1_padded(64) = 256 bytes
    let mut output = [0u8; 256];
    #[expect(clippy::indexing_slicing, reason = "result is 192 bytes")]
    {
        output[16..64].copy_from_slice(&result[0..48]);
        output[80..128].copy_from_slice(&result[48..96]);
        output[144..192].copy_from_slice(&result[96..144]);
        output[208..256].copy_from_slice(&result[144..192]);
    }
    Ok(Bytes::copy_from_slice(&output))
}
