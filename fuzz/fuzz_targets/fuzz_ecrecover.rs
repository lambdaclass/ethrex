//! Fuzz the ecrecover precompile with structured inputs
//!
//! ecrecover (0x01) recovers the Ethereum address from an ECDSA signature.
//! Input format: hash (32 bytes) || v (32 bytes) || r (32 bytes) || s (32 bytes) = 128 bytes
//!
//! This fuzzer tests:
//! - Valid signature formats
//! - Invalid v values (should be 27 or 28)
//! - Invalid r/s values (out of curve order)
//! - Truncated inputs
//! - Extended inputs

#![no_main]

use arbitrary::Arbitrary;
use bytes::Bytes;
use ethrex_common::types::Fork;
use ethrex_common::Address;
use ethrex_levm::precompiles::execute_precompile;
use libfuzzer_sys::fuzz_target;

/// Ecrecover precompile address
const ECRECOVER_ADDRESS: Address = Address([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
]);

/// Structured input for ecrecover fuzzing
#[derive(Arbitrary, Debug)]
struct EcrecoverInput {
    /// Message hash (32 bytes)
    hash: [u8; 32],
    /// Recovery id v - can be any value, but only 27/28 are valid
    v: [u8; 32],
    /// Signature r component
    r: [u8; 32],
    /// Signature s component
    s: [u8; 32],
    /// Whether to truncate the input
    truncate_at: Option<u8>,
    /// Extra bytes to append
    extra_bytes: Vec<u8>,
}

/// Raw bytes input for edge cases
#[derive(Arbitrary, Debug)]
enum FuzzInput {
    /// Structured ecrecover input
    Structured(EcrecoverInput),
    /// Raw arbitrary bytes
    Raw(Vec<u8>),
}

fuzz_target!(|input: FuzzInput| {
    let calldata = match input {
        FuzzInput::Structured(ec) => {
            let mut data = Vec::with_capacity(128 + ec.extra_bytes.len());
            data.extend_from_slice(&ec.hash);
            data.extend_from_slice(&ec.v);
            data.extend_from_slice(&ec.r);
            data.extend_from_slice(&ec.s);
            data.extend_from_slice(&ec.extra_bytes);

            // Optionally truncate
            if let Some(truncate) = ec.truncate_at {
                let truncate_len = (truncate as usize) % (data.len() + 1);
                data.truncate(truncate_len);
            }

            Bytes::from(data)
        }
        FuzzInput::Raw(data) => Bytes::from(data),
    };

    let mut gas_remaining: u64 = 10_000_000;

    // Execute ecrecover - should never panic
    let _ = execute_precompile(ECRECOVER_ADDRESS, &calldata, &mut gas_remaining, Fork::Prague);
});
