//! Fuzz target for block header decoding.
//!
//! This fuzzer tests that block header decoding never panics on arbitrary input.
//! Even malformed block header data should return an error rather than panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

use ethrex_common::types::BlockHeader;
use ethrex_rlp::decode::RLPDecode;

fuzz_target!(|data: &[u8]| {
    // Try to decode as a BlockHeader
    // This should never panic, only return Ok or Err
    let result = BlockHeader::decode(data);

    // If decode succeeds, try encoding back and verify consistency
    if let Ok(header) = result {
        use ethrex_rlp::encode::RLPEncode;
        let encoded = header.encode_to_vec();

        // Re-decode should succeed
        if let Ok(redecoded) = BlockHeader::decode(&encoded) {
            // Hash should be deterministic
            let hash1 = header.hash();
            let hash2 = redecoded.hash();
            assert_eq!(hash1, hash2, "Block header hash should be deterministic");
        }
    }
});
