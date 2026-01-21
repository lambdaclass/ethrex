//! Stack-allocated composite keys for database operations.
//!
//! These key types avoid heap allocations for fixed-size database keys in hot paths.

use ethrex_common::{H256, types::BlockHash};

/// 64-byte key for transaction locations: tx_hash (32) + block_hash (32)
///
/// Used in TRANSACTION_LOCATIONS table to index transactions by their hash
/// and the block they belong to.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct TransactionLocationKey([u8; 64]);

impl TransactionLocationKey {
    /// Creates a new transaction location key from transaction hash and block hash.
    #[inline]
    pub fn new(tx_hash: H256, block_hash: BlockHash) -> Self {
        let mut key = [0u8; 64];
        key[..32].copy_from_slice(tx_hash.as_bytes());
        key[32..].copy_from_slice(block_hash.as_bytes());
        Self(key)
    }
}

impl AsRef<[u8]> for TransactionLocationKey {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// 40-byte key for execution witnesses: block_number (8 BE) + block_hash (32)
///
/// Used in EXECUTION_WITNESSES table to store execution witnesses indexed by
/// block number and hash. Block number is stored in big-endian for proper
/// prefix iteration ordering.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct WitnessKey([u8; 40]);

impl WitnessKey {
    /// Creates a new witness key from block number and block hash.
    #[inline]
    pub fn new(block_number: u64, block_hash: &BlockHash) -> Self {
        let mut key = [0u8; 40];
        key[..8].copy_from_slice(&block_number.to_be_bytes());
        key[8..].copy_from_slice(block_hash.as_bytes());
        Self(key)
    }
}

impl AsRef<[u8]> for WitnessKey {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_location_key_construction() {
        let tx_hash = H256::from_low_u64_be(0x1234);
        let block_hash = H256::from_low_u64_be(0x5678);

        let key = TransactionLocationKey::new(tx_hash, block_hash);
        let bytes = key.as_ref();

        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[..32], tx_hash.as_bytes());
        assert_eq!(&bytes[32..], block_hash.as_bytes());
    }

    #[test]
    fn witness_key_construction() {
        let block_number: u64 = 12345;
        let block_hash = H256::from_low_u64_be(0xabcd);

        let key = WitnessKey::new(block_number, &block_hash);
        let bytes = key.as_ref();

        assert_eq!(bytes.len(), 40);
        assert_eq!(&bytes[..8], &block_number.to_be_bytes());
        assert_eq!(&bytes[8..], block_hash.as_bytes());
    }

    #[test]
    fn witness_key_ordering() {
        // Verify big-endian ordering allows proper prefix iteration
        let hash = H256::zero();
        let key1 = WitnessKey::new(100, &hash);
        let key2 = WitnessKey::new(200, &hash);

        assert!(key1.as_ref() < key2.as_ref());
    }
}
