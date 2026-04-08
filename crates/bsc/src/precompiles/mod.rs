use ethereum_types::H160;
use ethrex_common::Address;

pub mod bls_verify;
pub mod cometbft_validate;
pub mod double_sign;
pub mod iavl_merkle_proof;
pub mod p256_verify;
pub mod secp256k1_recover;
pub mod tm_header_validate;

/// Error type for BSC precompile execution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrecompileError {
    /// Gas limit exceeded.
    #[error("not enough gas to execute BSC precompile")]
    NotEnoughGas,
    /// Input is malformed or has the wrong length.
    #[error("invalid input for BSC precompile")]
    InvalidInput,
    /// Precompile logic failed (e.g. signature verification failed in a way
    /// that should revert the call rather than return an empty/zero output).
    #[error("BSC precompile execution reverted")]
    ExecutionReverted,
    /// Address not in the BSC precompile set.
    #[error("address is not a BSC precompile")]
    NotAPrecompile,
    /// Precompile is recognised but not yet implemented.
    #[error("BSC precompile not yet implemented")]
    NotImplemented,
}

/// All BSC-specific precompile addresses paired with their names.
pub const BSC_PRECOMPILE_ADDRESSES: &[(Address, &str)] = &[
    (address(0x64), "tmHeaderValidate"),
    (address(0x65), "iavlMerkleProofValidate"),
    (address(0x66), "blsSignatureVerify"),
    (address(0x67), "cometBFTLightBlockValidate"),
    (address(0x68), "verifyDoubleSignEvidence"),
    (address(0x69), "secp256k1SignatureRecover"),
    (address(0x0100), "p256Verify"),
];

/// Construct a 20-byte `Address` from a single `u16` value stored in the
/// last two bytes (big-endian), all other bytes zero.
const fn address(val: u16) -> Address {
    let [hi, lo] = val.to_be_bytes();
    let mut bytes = [0u8; 20];
    bytes[18] = hi;
    bytes[19] = lo;
    H160(bytes)
}

/// Convert a 20-byte address to its `u64` representation using the last two
/// bytes (big-endian).  Values ≤ 0xFFFF fit; anything beyond is not a BSC
/// precompile address and will miss the match in `run_bsc_precompile`.
#[inline]
fn address_to_u64(addr: &Address) -> u64 {
    let hi = addr.0[18] as u64;
    let lo = addr.0[19] as u64;
    (hi << 8) | lo
}

/// Returns `true` if `address` belongs to the BSC-specific precompile set.
pub fn is_bsc_precompile(address: &Address) -> bool {
    // First 18 bytes must all be zero.
    if address.0[..18] != [0u8; 18] {
        return false;
    }
    matches!(
        address_to_u64(address),
        0x64 | 0x65 | 0x66 | 0x67 | 0x68 | 0x69 | 0x100
    )
}

/// Execute a BSC precompile and return `(gas_used, output)`.
///
/// `gas_limit` is the maximum gas available for this call.  On success the
/// returned `gas_used` value is always `≤ gas_limit`.
pub fn run_bsc_precompile(
    address: &Address,
    input: &[u8],
    gas_limit: u64,
) -> Result<(u64, Vec<u8>), PrecompileError> {
    match address_to_u64(address) {
        0x64 => tm_header_validate::run(input, gas_limit),
        0x65 => iavl_merkle_proof::run(input, gas_limit),
        0x66 => bls_verify::run(input, gas_limit),
        0x67 => cometbft_validate::run(input, gas_limit),
        0x68 => double_sign::run(input, gas_limit),
        0x69 => secp256k1_recover::run(input, gas_limit),
        0x100 => p256_verify::run(input, gas_limit),
        _ => Err(PrecompileError::NotAPrecompile),
    }
}
