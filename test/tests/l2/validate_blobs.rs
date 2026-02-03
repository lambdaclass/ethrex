//! Fast validation test to check that blobs under fixtures/blobs are compatible with the l2 genesis file (l2.json).
//!
//! This test will fail fast when the genesis file is modified but the blobs haven't been regenerated.
//!
//! When this test fails, regenerate blobs following: docs/developers/l2/state-reconstruction-blobs.md

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use ethrex_common::types::{BYTES_PER_BLOB, Block, Genesis, bytes_from_blob};
use ethrex_rlp::decode::RLPDecode;
use std::fs::File;
use std::io::BufReader;

use super::common::workspace_root;

/// Validates that the fixture blobs are compatible with the current genesis file.
///
/// The first block in the first blob must have a `parent_hash` that matches
/// the genesis block hash. If the genesis file is modified (changing the state root
/// or any header field), the genesis block hash changes, and the blobs become stale.
///
/// This test catches stale blobs fast rather than waiting for the full state-diff-test workflow to fail.
#[test]
fn validate_blobs_match_genesis() {
    let genesis_path = workspace_root().join("fixtures/genesis/l2.json");
    let first_blob_path = workspace_root().join("fixtures/blobs/1-1.blob");

    // Load genesis file and compute the genesis block hash
    let genesis_file =
        File::open(&genesis_path).expect("Failed to open genesis file");
    let reader = BufReader::new(genesis_file);
    let genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let genesis_block = genesis.get_block();
    let genesis_hash = genesis_block.hash();

    // Read the first blob file
    let blob_bytes = std::fs::read(&first_blob_path).expect("Failed to read first blob file");

    assert_eq!(
        blob_bytes.len(),
        BYTES_PER_BLOB,
        "Invalid blob size. Expected {} bytes, got {} bytes",
        BYTES_PER_BLOB,
        blob_bytes.len()
    );

    // Decode the blob to extract block data
    let decoded_blob = bytes_from_blob(blob_bytes.into());

    // First 8 bytes are the block count
    let blocks_count = u64::from_be_bytes(
        decoded_blob[0..8]
            .try_into()
            .expect("Failed to get block count from blob"),
    );

    assert!(
        blocks_count > 0,
        "Blob contains no blocks. Expected at least 1 block."
    );

    // Decode the first block to get its parent_hash
    let (first_block, _) = Block::decode_unfinished(&decoded_blob[8..])
        .expect("Failed to decode first block from blob");

    let blob_parent_hash = first_block.header.parent_hash;

    // The first block's parent_hash must match the genesis block hash
    assert_eq!(
        blob_parent_hash, genesis_hash,
        "\n\n\
        ========================================================\n\
        BLOB FIXTURES ARE STALE!\n\
        ========================================================\n\n\
        The genesis file has been modified, but the blob fixtures\n\
        haven't been regenerated.\n\n\
        Expected parent_hash: {genesis_hash:#x}\n\
        Found parent_hash:    {blob_parent_hash:#x}\n\n\
        To fix this, regenerate the blobs following:\n\
        docs/developers/l2/state-reconstruction-blobs.md\n\
        ========================================================\n"
    );
}
