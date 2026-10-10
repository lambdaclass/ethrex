//! Fixture conformance for `fixtures/blobs/` (L2 state reconstruction).
//!
//! Decodes every blob and validates each block's body against
//! [`validate_block_body`], and checks that the first block links to the L2
//! genesis. The workspace test suite runs on every non-docs PR
//! (`pr-main_l1.yaml`), so fixture/schema drift is caught at PR time instead
//! of surfacing on `main` in the (path-filtered) L2 State Reconstruction job
//! — e.g. a `BlockBody`/`BlockHeader` schema change in `crates/common` that
//! leaves the blobs undecodable or invalid.
//!
//! When this fails, the fixtures are stale: regenerate them following
//! docs/workflows/regenerate-blobs.md (or migrate them in place when the
//! change is encoding-only, as in #7063).

use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::types::{
    BYTES_PER_BLOB, Block, Genesis, bytes_from_blob, validate_block_body,
};
use ethrex_common::NativeCrypto;
use ethrex_rlp::decode::RLPDecode;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn blob_fixtures_decode_and_pass_block_validation() -> Result<(), Box<dyn Error>> {
    let root = workspace_root();

    // Genesis linkage: the first block of the first blob must point at the
    // L2 genesis block hash (catches fixtures left stale by a genesis change).
    let genesis_file = fs::File::open(root.join("fixtures/genesis/l2.json"))?;
    let genesis: Genesis = serde_json::from_reader(genesis_file)?;
    let genesis_hash = genesis.get_block().hash();

    let blobs_dir = root.join("fixtures/blobs");
    let mut entries: Vec<_> = fs::read_dir(&blobs_dir)?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    assert!(!entries.is_empty(), "no blob fixtures in {blobs_dir:?}");

    let mut total_blocks = 0u64;
    for (file_idx, entry) in entries.iter().enumerate() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("blob") {
            continue;
        }
        let raw = fs::read(&path)?;
        assert_eq!(
            raw.len(),
            BYTES_PER_BLOB,
            "{}: invalid blob size {}",
            path.display(),
            raw.len()
        );
        let data = bytes_from_blob(raw.into());
        let blocks_count = u64::from_be_bytes(data[0..8].try_into()?);
        assert!(blocks_count > 0, "{}: blob contains no blocks", path.display());

        let mut buf = &data[8..];
        for block_idx in 0..blocks_count {
            let (block, rest) = Block::decode_unfinished(buf)?;
            buf = rest;

            if file_idx == 0 && block_idx == 0 {
                assert_eq!(
                    block.header.parent_hash, genesis_hash,
                    "stale blob fixtures: first block's parent is not the L2 \
                     genesis hash — see docs/workflows/regenerate-blobs.md"
                );
            }

            validate_block_body(&block.header, &block.body, &NativeCrypto).map_err(|e| {
                format!(
                    "{}: block {} ({}) fails validate_block_body: {e}. \
                     Blob fixtures are stale relative to the block schema — \
                     see docs/workflows/regenerate-blobs.md",
                    path.display(),
                    block.header.number,
                    block.hash(),
                )
            })?;
            total_blocks += 1;
        }

        // The fee-config trailer must decode fully as well.
        for _ in 0..blocks_count {
            let (consumed, _) = FeeConfig::decode(buf)?;
            buf = &buf[consumed..];
        }
    }

    assert!(total_blocks > 0, "no blocks decoded from blob fixtures");
    Ok(())
}
