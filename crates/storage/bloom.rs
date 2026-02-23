use ethrex_common::{Address, H256};
use std::sync::atomic::{AtomicU64, Ordering};

/// Number of bits in the bloom filter: 2^30 = ~1 billion bits = 128 MB.
const NUM_BITS: usize = 1 << 30;

/// Number of AtomicU64 words needed: NUM_BITS / 64.
const NUM_WORDS: usize = NUM_BITS / 64;

/// Bit mask to convert a hash probe into a bit index within NUM_BITS.
const BIT_INDEX_MASK: u64 = (NUM_BITS as u64) - 1;

/// Lock-free bloom filter for tracking which (address, storage_slot) pairs
/// have ever been observed in storage.
///
/// Used as a fast negative lookup: if the bloom says a slot is absent, we can
/// skip the trie traversal and return zero immediately. False positives are
/// harmless (they just fall through to the normal DB path).
///
/// The filter uses 3 hash probes derived from keccak256(address ++ slot).
/// It is insert-only (no delete) and not persisted to disk.
pub struct StorageBloom {
    bits: Box<[AtomicU64]>,
}

impl StorageBloom {
    pub fn new() -> Self {
        let mut v = Vec::with_capacity(NUM_WORDS);
        for _ in 0..NUM_WORDS {
            v.push(AtomicU64::new(0));
        }
        Self {
            bits: v.into_boxed_slice(),
        }
    }

    /// Insert an (address, slot) pair into the bloom filter.
    #[inline]
    pub fn insert(&self, address: &Address, slot: &H256) {
        let hash = Self::hash_key(address, slot);
        let (i0, m0) = Self::probe(&hash, 0);
        let (i1, m1) = Self::probe(&hash, 1);
        let (i2, m2) = Self::probe(&hash, 2);
        self.bits[i0].fetch_or(m0, Ordering::Relaxed);
        self.bits[i1].fetch_or(m1, Ordering::Relaxed);
        self.bits[i2].fetch_or(m2, Ordering::Relaxed);
    }

    /// Check if an (address, slot) pair is definitely absent.
    ///
    /// Returns `true` if the pair is definitely NOT in the filter (safe to
    /// skip the DB lookup). Returns `false` if the pair MIGHT be present
    /// (must do the full lookup).
    #[inline]
    pub fn definitely_absent(&self, address: &Address, slot: &H256) -> bool {
        let hash = Self::hash_key(address, slot);
        let (i0, m0) = Self::probe(&hash, 0);
        let (i1, m1) = Self::probe(&hash, 1);
        let (i2, m2) = Self::probe(&hash, 2);
        (self.bits[i0].load(Ordering::Relaxed) & m0) == 0
            || (self.bits[i1].load(Ordering::Relaxed) & m1) == 0
            || (self.bits[i2].load(Ordering::Relaxed) & m2) == 0
    }

    /// Compute keccak256(address ++ slot) for probe derivation.
    #[inline]
    fn hash_key(address: &Address, slot: &H256) -> [u8; 32] {
        let mut buf = [0u8; 52]; // 20 bytes address + 32 bytes slot
        buf[..20].copy_from_slice(address.as_bytes());
        buf[20..].copy_from_slice(slot.as_bytes());
        let h: H256 = ethrex_common::utils::keccak(&buf);
        h.0
    }

    /// Extract the word index and bit mask for the k-th probe from the hash.
    ///
    /// Probe 0 uses bytes 0..4, probe 1 uses bytes 4..8, probe 2 uses bytes 8..12.
    /// Each 4-byte chunk gives a 30-bit index into the bit array.
    #[inline]
    fn probe(hash: &[u8; 32], k: usize) -> (usize, u64) {
        let offset = k * 4;
        let raw = u32::from_le_bytes([
            hash[offset],
            hash[offset + 1],
            hash[offset + 2],
            hash[offset + 3],
        ]);
        let bit_index = (raw as u64) & BIT_INDEX_MASK;
        let word_index = (bit_index / 64) as usize;
        let bit_within_word = bit_index % 64;
        (word_index, 1u64 << bit_within_word)
    }
}

// AtomicU64 is inherently Send+Sync, and Box<[AtomicU64]> is too,
// so the derived impls are sound. We only use Relaxed ordering since
// we tolerate stale reads (false negatives degrade to the normal path).

impl std::fmt::Debug for StorageBloom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageBloom")
            .field("size_bits", &NUM_BITS)
            .field("num_probes", &3)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_check() {
        let bloom = StorageBloom::new();
        let addr = Address::from_low_u64_be(42);
        let slot = H256::from_low_u64_be(7);

        bloom.insert(&addr, &slot);
        assert!(
            !bloom.definitely_absent(&addr, &slot),
            "inserted pair must not be reported as absent"
        );
    }

    #[test]
    fn absent_for_unknown_slot() {
        let bloom = StorageBloom::new();
        let addr = Address::from_low_u64_be(1);
        let slot = H256::from_low_u64_be(999);

        // Empty bloom should report everything as absent
        assert!(
            bloom.definitely_absent(&addr, &slot),
            "empty bloom should report unknown slots as absent"
        );
    }

    #[test]
    fn different_slots_are_independent() {
        let bloom = StorageBloom::new();
        let addr = Address::from_low_u64_be(1);
        let slot_a = H256::from_low_u64_be(100);
        let slot_b = H256::from_low_u64_be(200);

        bloom.insert(&addr, &slot_a);
        assert!(!bloom.definitely_absent(&addr, &slot_a));
        // slot_b was not inserted — it should likely be absent
        // (there's a tiny FP chance, but with 1B bits and 1 entry it's negligible)
        assert!(bloom.definitely_absent(&addr, &slot_b));
    }

    #[test]
    fn different_addresses_are_independent() {
        let bloom = StorageBloom::new();
        let addr_a = Address::from_low_u64_be(1);
        let addr_b = Address::from_low_u64_be(2);
        let slot = H256::from_low_u64_be(42);

        bloom.insert(&addr_a, &slot);
        assert!(!bloom.definitely_absent(&addr_a, &slot));
        assert!(bloom.definitely_absent(&addr_b, &slot));
    }

    #[test]
    fn bulk_insert_no_false_negatives() {
        let bloom = StorageBloom::new();
        let addr = Address::from_low_u64_be(0xDEAD);

        // Insert 1000 slots and verify none are reported as absent
        for i in 0..1000u64 {
            let slot = H256::from_low_u64_be(i);
            bloom.insert(&addr, &slot);
        }
        for i in 0..1000u64 {
            let slot = H256::from_low_u64_be(i);
            assert!(
                !bloom.definitely_absent(&addr, &slot),
                "inserted slot {i} must not be absent"
            );
        }
    }
}
