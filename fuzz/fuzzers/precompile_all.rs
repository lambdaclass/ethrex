//! Fuzz all EVM precompiles with arbitrary calldata
//!
//! This fuzzer tests all precompile contracts by dispatching random calldata
//! to each precompile address. The goal is to find panics, overflows, or
//! unexpected behavior in the precompile implementations.

#![no_main]

use arbitrary::Arbitrary;
use bytes::Bytes;
use ethrex_common::types::Fork;
use ethrex_common::H160;
use ethrex_levm::precompiles::execute_precompile;
use libfuzzer_sys::fuzz_target;

/// Precompile addresses (last byte for addresses 0x01-0x11, special case for P256VERIFY at 0x0100)
const PRECOMPILE_ADDRESSES: &[u8] = &[
    0x01, // ecrecover
    0x02, // sha256
    0x03, // ripemd160
    0x04, // identity
    0x05, // modexp
    0x06, // ecadd (BN254)
    0x07, // ecmul (BN254)
    0x08, // ecpairing (BN254)
    0x09, // blake2f
    0x0a, // point_evaluation (KZG)
    0x0b, // bls12_g1add
    0x0c, // bls12_g1msm
    0x0d, // bls12_g2add
    0x0e, // bls12_g2msm
    0x0f, // bls12_pairing_check
    0x10, // bls12_map_fp_to_g1
    0x11, // bls12_map_fp2_to_g2
];

/// Input for the precompile fuzzer
#[derive(Arbitrary, Debug)]
struct PrecompileInput {
    /// Index into PRECOMPILE_ADDRESSES (will be modulo'd)
    precompile_index: u8,
    /// Raw calldata to pass to the precompile
    calldata: Vec<u8>,
    /// Whether to use Prague fork (enables BLS12 precompiles)
    use_prague: bool,
}

fn make_precompile_address(last_byte: u8) -> H160 {
    let mut addr = [0u8; 20];
    addr[19] = last_byte;
    H160(addr)
}

fn make_p256_address() -> H160 {
    let mut addr = [0u8; 20];
    addr[18] = 0x01;
    addr[19] = 0x00;
    H160(addr)
}

fuzz_target!(|input: PrecompileInput| {
    let fork = if input.use_prague {
        Fork::Prague
    } else {
        Fork::Cancun
    };

    // Select precompile address
    let precompile_idx = (input.precompile_index as usize) % PRECOMPILE_ADDRESSES.len();
    let address = make_precompile_address(PRECOMPILE_ADDRESSES[precompile_idx]);

    let calldata = Bytes::from(input.calldata);

    // Use a generous gas limit to allow the precompile to run
    let mut gas_remaining: u64 = 10_000_000;

    // Execute the precompile - we expect it to either succeed or return a proper error,
    // but never panic or overflow
    let _ = execute_precompile(address, &calldata, &mut gas_remaining, fork);
});
