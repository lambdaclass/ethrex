//! Fuzz target for RLP decoding.
//!
//! This fuzzer tests that the RLP decoder never panics on arbitrary input.
//! Even invalid RLP data should return an error rather than panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

use bytes::Bytes;
use ethrex_rlp::decode::RLPDecode;
use ethereum_types::{Address, H256, U256};

fuzz_target!(|data: &[u8]| {
    // Try to decode as various types - should never panic, only return errors

    // Basic integer types
    let _ = u8::decode(data);
    let _ = u16::decode(data);
    let _ = u32::decode(data);
    let _ = u64::decode(data);
    let _ = u128::decode(data);

    // Boolean
    let _ = bool::decode(data);

    // String
    let _ = String::decode(data);

    // Bytes
    let _ = Bytes::decode(data);

    // Fixed-size arrays
    let _ = <[u8; 20]>::decode(data);
    let _ = <[u8; 32]>::decode(data);

    // Ethereum types
    let _ = U256::decode(data);
    let _ = H256::decode(data);
    let _ = Address::decode(data);

    // Lists
    let _ = Vec::<u8>::decode(data);
    let _ = Vec::<u64>::decode(data);
    let _ = Vec::<Vec<u8>>::decode(data);

    // Tuples
    let _ = <(u8, u8)>::decode(data);
    let _ = <(u64, u64)>::decode(data);
    let _ = <(u64, String)>::decode(data);
});
