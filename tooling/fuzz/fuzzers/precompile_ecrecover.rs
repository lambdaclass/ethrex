//! Fuzz the ecrecover precompile with structured inputs
//!
//! ecrecover (0x01) recovers the Ethereum address from an ECDSA signature.
//! Input format: hash (32 bytes) || v (32 bytes) || r (32 bytes) || s (32 bytes) = 128 bytes

#![no_main]

use arbitrary::Arbitrary;
use bytes::Bytes;
use ethrex_common::types::Fork;
use ethrex_common::H160;
use ethrex_levm::precompiles::execute_precompile;
use libfuzzer_sys::fuzz_target;

/// Ecrecover precompile address (0x01)
const ECRECOVER_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
]);

#[derive(Arbitrary, Debug)]
struct EcrecoverInput {
    hash: [u8; 32],
    v: [u8; 32],
    r: [u8; 32],
    s: [u8; 32],
    truncate_at: Option<u8>,
    extra_bytes: Vec<u8>,
}

#[derive(Arbitrary, Debug)]
enum FuzzInput {
    Structured(EcrecoverInput),
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

            if let Some(truncate) = ec.truncate_at {
                let truncate_len = (truncate as usize) % (data.len() + 1);
                data.truncate(truncate_len);
            }

            Bytes::from(data)
        }
        FuzzInput::Raw(data) => Bytes::from(data),
    };

    let mut gas_remaining: u64 = 10_000_000;
    let _ = execute_precompile(ECRECOVER_ADDRESS, &calldata, &mut gas_remaining, Fork::Prague);
});
