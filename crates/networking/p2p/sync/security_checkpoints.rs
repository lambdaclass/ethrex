//! Security checkpoints for header backfill validation.
//!
//! This module provides hardcoded block hashes at known heights to protect against
//! long-range attacks during header backfill. These checkpoints are verified during
//! the backfill process to ensure we're downloading headers from the canonical chain.
//!
//! Checkpoints are sourced from well-known block explorers and client implementations.

use ethrex_common::H256;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;

/// Mainnet chain ID
pub const MAINNET_CHAIN_ID: u64 = 1;

/// Mainnet security checkpoints - well-known block hashes at specific heights.
/// These are used to validate headers during backfill to prevent long-range attacks.
///
/// Sources: etherscan.io, Ethereum specs
pub static MAINNET_CHECKPOINTS: LazyLock<HashMap<u64, H256>> = LazyLock::new(|| {
    let mut checkpoints = HashMap::new();

    // Genesis block - canonical Ethereum genesis hash
    checkpoints.insert(
        0,
        H256::from_str("0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3")
            .expect("valid checkpoint hash"),
    );

    // Block 1 - first block after genesis
    checkpoints.insert(
        1,
        H256::from_str("0x88e96d4537bea4d9c05d12549907b32561d3bf31f45aae734cdc119f13406cb6")
            .expect("valid checkpoint hash"),
    );

    // Block 1,000,000
    checkpoints.insert(
        1_000_000,
        H256::from_str("0x8e38b4dbf6b11fcc3b9dee84fb7986e29ca0a02cecd8977c161ff7333329681e")
            .expect("valid checkpoint hash"),
    );

    // DAO fork block (1,920,000)
    checkpoints.insert(
        1_920_000,
        H256::from_str("0x4985f5ca3d2afbec36529aa96f74de3cc10a2a4a6c44f2157a57d2c6059a11bb")
            .expect("valid checkpoint hash"),
    );

    // Block 5,000,000
    checkpoints.insert(
        5_000_000,
        H256::from_str("0x7d5a4369273c723454ac137f48a4f142b097aa2779464e6505f1b1c5e37b5382")
            .expect("valid checkpoint hash"),
    );

    // Block 10,000,000
    checkpoints.insert(
        10_000_000,
        H256::from_str("0xaa20f7bde5be60603f11a45fc4923aab7552be775403fc00c2e6b805e6297dbe")
            .expect("valid checkpoint hash"),
    );

    // The Merge (block 15,537,394) - Paris upgrade / terminal PoW block
    checkpoints.insert(
        15_537_394,
        H256::from_str("0x56a9bb0302da44b8c0b3df540781424684c3af04d0b7a38d72842b762076a664")
            .expect("valid checkpoint hash"),
    );

    // Block 16,000,000
    checkpoints.insert(
        16_000_000,
        H256::from_str("0x3dc4ef568ae2635db1419c5fec55c4a9322c05302ae527cd40bff380c1d465dd")
            .expect("valid checkpoint hash"),
    );

    // Block 18,000,000
    checkpoints.insert(
        18_000_000,
        H256::from_str("0x78dfaf1a28cde6fcc6d4d78a59c23bc1e0e0cbb4ee62c53d0bad4e9aa2fce8bf")
            .expect("valid checkpoint hash"),
    );

    // Block 20,000,000
    checkpoints.insert(
        20_000_000,
        H256::from_str("0x0bef22d6df8e0a17b8d5aaefb7dd7e0b297f8fcc9e9e2a0fb6bc929f3a7fcf1f")
            .expect("valid checkpoint hash"),
    );

    checkpoints
});

/// Sepolia testnet checkpoints
pub static SEPOLIA_CHECKPOINTS: LazyLock<HashMap<u64, H256>> = LazyLock::new(|| {
    let mut checkpoints = HashMap::new();

    // Sepolia genesis
    checkpoints.insert(
        0,
        H256::from_str("0x25a5cc106eea7138acab33231d7160d69cb777ee0c2c553fcddf5138993e6dd9")
            .expect("valid checkpoint hash"),
    );

    checkpoints
});

/// Holesky testnet checkpoints
pub static HOLESKY_CHECKPOINTS: LazyLock<HashMap<u64, H256>> = LazyLock::new(|| {
    let mut checkpoints = HashMap::new();

    // Holesky genesis
    checkpoints.insert(
        0,
        H256::from_str("0xb5f7f912443c940f21fd611f12828d75b534364ed9e95ca4e307729a4661bde4")
            .expect("valid checkpoint hash"),
    );

    checkpoints
});

/// Returns the security checkpoints for a given chain ID.
pub fn get_checkpoints_for_chain(chain_id: u64) -> Option<&'static HashMap<u64, H256>> {
    match chain_id {
        1 => Some(&MAINNET_CHECKPOINTS),
        11155111 => Some(&SEPOLIA_CHECKPOINTS),
        17000 => Some(&HOLESKY_CHECKPOINTS),
        _ => None,
    }
}

/// Error returned when checkpoint validation fails.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CheckpointError {
    #[error("Checkpoint mismatch at block {block_number}: expected {expected}, got {actual}")]
    Mismatch {
        block_number: u64,
        expected: H256,
        actual: H256,
    },
}

/// Validates a batch of headers against security checkpoints.
///
/// Returns Ok(()) if all headers pass validation, or an error if any checkpoint mismatches.
/// Headers at heights without checkpoints are allowed to pass.
pub fn validate_headers_against_checkpoints(
    headers: &[ethrex_common::types::BlockHeader],
    chain_id: u64,
) -> Result<(), CheckpointError> {
    let Some(checkpoints) = get_checkpoints_for_chain(chain_id) else {
        // No checkpoints for this chain, skip validation
        return Ok(());
    };

    for header in headers {
        if let Some(expected_hash) = checkpoints.get(&header.number) {
            let actual_hash = header.hash();
            if actual_hash != *expected_hash {
                return Err(CheckpointError::Mismatch {
                    block_number: header.number,
                    expected: *expected_hash,
                    actual: actual_hash,
                });
            }
        }
    }

    Ok(())
}

/// Returns the closest checkpoint at or below the given block number.
/// Useful for determining the last verified point during backfill.
pub fn get_closest_checkpoint_below(block_number: u64, chain_id: u64) -> Option<(u64, H256)> {
    let checkpoints = get_checkpoints_for_chain(chain_id)?;

    checkpoints
        .iter()
        .filter(|(height, _)| **height <= block_number)
        .max_by_key(|(height, _)| *height)
        .map(|(height, hash)| (*height, *hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mainnet_genesis_checkpoint() {
        let checkpoints = get_checkpoints_for_chain(1).expect("mainnet checkpoints exist");
        assert!(checkpoints.contains_key(&0));
        assert_eq!(
            checkpoints.get(&0).expect("genesis checkpoint exists"),
            &H256::from_str("0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3")
                .expect("valid hash")
        );
    }

    #[test]
    fn test_unknown_chain_returns_none() {
        assert!(get_checkpoints_for_chain(999999).is_none());
    }

    #[test]
    fn test_validate_empty_headers() {
        assert!(validate_headers_against_checkpoints(&[], 1).is_ok());
    }

    #[test]
    fn test_closest_checkpoint() {
        // Should return the closest checkpoint at or below block 5,500,000
        let result = get_closest_checkpoint_below(5_500_000, 1);
        assert!(result.is_some());
        let (height, _) = result.expect("checkpoint should exist");
        assert!(height <= 5_500_000);
    }
}
