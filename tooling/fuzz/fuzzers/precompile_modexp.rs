//! Fuzz the modexp precompile with structured inputs
//!
//! modexp (0x05) computes base^exp mod modulus.
//! Input format:
//!   base_length (32 bytes) || exp_length (32 bytes) || mod_length (32 bytes) ||
//!   base (base_length bytes) || exp (exp_length bytes) || mod (mod_length bytes)
//!
//! This is a complex precompile that has historically had many edge cases:
//! - Zero modulus
//! - Very large exponents
//! - Length field overflows
//! - Mismatched lengths

#![no_main]

use arbitrary::Arbitrary;
use bytes::Bytes;
use ethrex_common::types::Fork;
use ethrex_common::H160;
use ethrex_levm::precompiles::execute_precompile;
use libfuzzer_sys::fuzz_target;

/// Modexp precompile address (0x05)
const MODEXP_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05,
]);

/// Structured input for modexp fuzzing
#[derive(Arbitrary, Debug)]
struct ModexpInput {
    /// Length of base (capped to prevent OOM)
    base_len: u16,
    /// Length of exponent (capped to prevent OOM)
    exp_len: u16,
    /// Length of modulus (capped to prevent OOM)
    mod_len: u16,
    /// Base value bytes
    base: Vec<u8>,
    /// Exponent value bytes
    exp: Vec<u8>,
    /// Modulus value bytes
    modulus: Vec<u8>,
    /// Whether to use exact lengths or fuzzer-provided lengths
    use_exact_lengths: bool,
}

/// Edge case inputs for modexp
#[derive(Arbitrary, Debug)]
struct ModexpEdgeCases {
    /// Use zero-length fields
    zero_base_len: bool,
    zero_exp_len: bool,
    zero_mod_len: bool,
    /// Use maximum u256 for lengths (should be handled gracefully)
    max_length_field: bool,
    /// Raw data after header
    data: Vec<u8>,
}

#[derive(Arbitrary, Debug)]
enum FuzzInput {
    /// Structured modexp input
    Structured(ModexpInput),
    /// Edge case testing
    EdgeCases(ModexpEdgeCases),
    /// Raw arbitrary bytes
    Raw(Vec<u8>),
}

fn u256_from_usize(value: usize) -> [u8; 32] {
    let mut result = [0u8; 32];
    let bytes = value.to_be_bytes();
    result[32 - bytes.len()..].copy_from_slice(&bytes);
    result
}

fuzz_target!(|input: FuzzInput| {
    let calldata = match input {
        FuzzInput::Structured(modexp) => {
            // Cap lengths to prevent OOM (max ~4KB each)
            let base_len = (modexp.base_len as usize).min(4096);
            let exp_len = (modexp.exp_len as usize).min(4096);
            let mod_len = (modexp.mod_len as usize).min(4096);

            let mut data = Vec::new();

            if modexp.use_exact_lengths {
                // Use actual data lengths
                data.extend_from_slice(&u256_from_usize(modexp.base.len()));
                data.extend_from_slice(&u256_from_usize(modexp.exp.len()));
                data.extend_from_slice(&u256_from_usize(modexp.modulus.len()));
            } else {
                // Use fuzzer-provided lengths (may not match actual data)
                data.extend_from_slice(&u256_from_usize(base_len));
                data.extend_from_slice(&u256_from_usize(exp_len));
                data.extend_from_slice(&u256_from_usize(mod_len));
            }

            data.extend_from_slice(&modexp.base);
            data.extend_from_slice(&modexp.exp);
            data.extend_from_slice(&modexp.modulus);

            Bytes::from(data)
        }
        FuzzInput::EdgeCases(edge) => {
            let mut data = Vec::new();

            if edge.max_length_field {
                // Try to use very large length values
                data.extend_from_slice(&[0xff; 32]); // base_len = max
                data.extend_from_slice(&[0xff; 32]); // exp_len = max
                data.extend_from_slice(&[0xff; 32]); // mod_len = max
            } else {
                let base_len = if edge.zero_base_len { 0 } else { edge.data.len().min(100) };
                let exp_len = if edge.zero_exp_len { 0 } else { edge.data.len().min(100) };
                let mod_len = if edge.zero_mod_len { 0 } else { edge.data.len().min(100) };

                data.extend_from_slice(&u256_from_usize(base_len));
                data.extend_from_slice(&u256_from_usize(exp_len));
                data.extend_from_slice(&u256_from_usize(mod_len));
            }

            data.extend_from_slice(&edge.data);

            Bytes::from(data)
        }
        FuzzInput::Raw(data) => Bytes::from(data),
    };

    let mut gas_remaining: u64 = 100_000_000; // Modexp can use a lot of gas

    // Execute modexp - should never panic even with malformed inputs
    let _ = execute_precompile(MODEXP_ADDRESS, &calldata, &mut gas_remaining, Fork::Prague);
});
