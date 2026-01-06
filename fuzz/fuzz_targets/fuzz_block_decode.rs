//! Fuzz target for block decoding.
//!
//! This fuzzer tests that block decoding never panics on arbitrary input.
//! Even malformed block data should return an error rather than panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

use ethrex_common::types::Block;
use ethrex_rlp::decode::RLPDecode;

fuzz_target!(|data: &[u8]| {
    // Try to decode as a Block
    // This should never panic, only return Ok or Err
    let _ = Block::decode(data);
});
