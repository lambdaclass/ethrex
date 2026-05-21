//! One-shot diagnostic: try to SSZ-decode a captured /engine/v4/payloads body
//! using my NewPayloadV4Request and print the exact error.
//!
//!     cargo run -p ethrex-rpc --example decode_v4_body -- /tmp/v4_body.bin

use ethrex_rpc::engine_rest::types::new_payload::NewPayloadV4Request;
use libssz::SszDecode;
use std::env;
use std::fs;

fn main() {
    let path = env::args().nth(1).expect("usage: decode_v4_body <path>");
    let bytes = fs::read(&path).expect("read body");
    println!("body len: {} bytes", bytes.len());

    // Show the first 44 bytes (the SSZ container fixed part for NewPayloadV4Request)
    let head = &bytes[..bytes.len().min(48)];
    println!("first 48 bytes: {}", hex::encode(head));

    // Decode offsets
    if bytes.len() >= 44 {
        let payload_off = u32::from_le_bytes(bytes[0..4].try_into().expect("4 bytes at [0..4]"));
        let hashes_off = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes at [4..8]"));
        let beacon = &bytes[8..40];
        let requests_off =
            u32::from_le_bytes(bytes[40..44].try_into().expect("4 bytes at [40..44]"));
        println!("payload_off    = {payload_off}");
        println!("hashes_off     = {hashes_off}");
        println!("beacon_root    = 0x{}", hex::encode(beacon));
        println!("requests_off   = {requests_off}");
    }

    println!("\n=== attempt SSZ decode as NewPayloadV4Request ===");
    match NewPayloadV4Request::from_ssz_bytes(&bytes) {
        Ok(req) => {
            println!(
                "OK decoded: block_number={} block_hash=0x{}",
                req.execution_payload.block_number,
                hex::encode(req.execution_payload.block_hash),
            );
            println!(
                "  tx_count={}, withdrawals={}, exec_requests={}, expected_blob_hashes={}",
                req.execution_payload.transactions.len(),
                req.execution_payload.withdrawals.len(),
                req.execution_requests.len(),
                req.expected_blob_versioned_hashes.len(),
            );
        }
        Err(e) => {
            println!("DECODE ERROR: {e:?}");
        }
    }
}
