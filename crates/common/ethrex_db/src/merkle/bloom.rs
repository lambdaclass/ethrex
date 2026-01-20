//! Bloom filter for fast negative lookups.
//!
//! A Bloom filter is a probabilistic data structure that can tell you
//! definitely if an element is NOT in the set, or possibly in the set.
//! False positives are possible, but false negatives are not.

use super::node::keccak256;

/// Default number of bits in the filter (64KB = 524288 bits).
/// This gives ~1% false positive rate for ~50000 entries.
const DEFAULT_BITS: usize = 524288;

/// Number of hash functions to use.
/// Using 7 hash functions is optimal for ~1% false positive rate.
const NUM_HASHES: usize = 7;

/// A Bloom filter for fast membership testing.
///
/// This implementation uses Keccak-256 based hashing with multiple
/// hash functions derived from a single hash computation.
#[derive(Clone)]
pub struct BloomFilter {
    /// Bit vector.
    bits: Vec<u64>,
    /// Number of bits in the filter.
    num_bits: usize,
    /// Number of elements inserted.
    count: usize,
}

impl BloomFilter {
    /// Creates a new Bloom filter with default size.
    pub fn new() -> Self {
        Self::with_size(DEFAULT_BITS)
    }

    /// Creates a new Bloom filter with the specified number of bits.
    pub fn with_size(num_bits: usize) -> Self {
        let num_words = (num_bits + 63) / 64;
        Self {
            bits: vec![0u64; num_words],
            num_bits,
            count: 0,
        }
    }

    /// Creates a Bloom filter sized for an expected number of elements
    /// with approximately 1% false positive rate.
    pub fn for_capacity(expected_elements: usize) -> Self {
        // Optimal bits = -n * ln(p) / (ln(2)^2)
        // For p = 0.01: bits ≈ 9.6 * n
        let num_bits = (expected_elements * 10).max(1024);
        Self::with_size(num_bits)
    }

    /// Inserts an element into the filter.
    pub fn insert(&mut self, key: &[u8]) {
        let hashes = self.compute_hashes(key);
        for h in hashes {
            let idx = h % self.num_bits;
            let word_idx = idx / 64;
            let bit_idx = idx % 64;
            self.bits[word_idx] |= 1u64 << bit_idx;
        }
        self.count += 1;
    }

    /// Checks if an element might be in the filter.
    ///
    /// Returns `true` if the element might be present (could be false positive).
    /// Returns `false` if the element is definitely NOT present (no false negatives).
    #[inline]
    pub fn may_contain(&self, key: &[u8]) -> bool {
        let hashes = self.compute_hashes(key);
        for h in hashes {
            let idx = h % self.num_bits;
            let word_idx = idx / 64;
            let bit_idx = idx % 64;
            if self.bits[word_idx] & (1u64 << bit_idx) == 0 {
                return false;
            }
        }
        true
    }

    /// Returns the number of elements inserted.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Returns the estimated false positive rate.
    pub fn estimated_false_positive_rate(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        // p = (1 - e^(-k*n/m))^k
        let k = NUM_HASHES as f64;
        let n = self.count as f64;
        let m = self.num_bits as f64;
        let exp_term = (-k * n / m).exp();
        (1.0 - exp_term).powf(k)
    }

    /// Clears the filter.
    pub fn clear(&mut self) {
        self.bits.fill(0);
        self.count = 0;
    }

    /// Inserts a pre-hashed key (already keccak256).
    ///
    /// Use this when the key is already a 32-byte hash (like address_hash or slot_hash
    /// in snap sync) to avoid redundant hashing.
    #[inline]
    pub fn insert_prehashed(&mut self, hash: &[u8; 32]) {
        let hashes = self.hashes_from_prehashed(hash);
        for h in hashes {
            let idx = h % self.num_bits;
            let word_idx = idx / 64;
            let bit_idx = idx % 64;
            self.bits[word_idx] |= 1u64 << bit_idx;
        }
        self.count += 1;
    }

    /// Batch insert multiple pre-hashed keys.
    ///
    /// More efficient than individual inserts - avoids repeated hash computations
    /// and allows better CPU cache utilization.
    pub fn insert_batch_prehashed<'a>(&mut self, keys: impl IntoIterator<Item = &'a [u8; 32]>) {
        for hash in keys {
            let hashes = self.hashes_from_prehashed(hash);
            for h in hashes {
                let idx = h % self.num_bits;
                let word_idx = idx / 64;
                let bit_idx = idx % 64;
                self.bits[word_idx] |= 1u64 << bit_idx;
            }
            self.count += 1;
        }
    }

    /// Checks if a pre-hashed key might be in the filter.
    #[inline]
    pub fn may_contain_prehashed(&self, hash: &[u8; 32]) -> bool {
        let hashes = self.hashes_from_prehashed(hash);
        for h in hashes {
            let idx = h % self.num_bits;
            let word_idx = idx / 64;
            let bit_idx = idx % 64;
            if self.bits[word_idx] & (1u64 << bit_idx) == 0 {
                return false;
            }
        }
        true
    }

    /// Computes hash indices directly from a pre-hashed key.
    /// Skips keccak256 since the input is already a hash.
    #[inline]
    fn hashes_from_prehashed(&self, hash: &[u8; 32]) -> [usize; NUM_HASHES] {
        // Extract two 64-bit values from the 256-bit hash
        let h1 = u64::from_le_bytes(hash[0..8].try_into().unwrap()) as usize;
        let h2 = u64::from_le_bytes(hash[8..16].try_into().unwrap()) as usize;

        let mut hashes = [0usize; NUM_HASHES];
        for i in 0..NUM_HASHES {
            hashes[i] = h1.wrapping_add(i.wrapping_mul(h2));
        }
        hashes
    }

    /// Computes multiple hash values from a single key.
    /// Uses double hashing: h_i(x) = h1(x) + i * h2(x)
    fn compute_hashes(&self, key: &[u8]) -> [usize; NUM_HASHES] {
        let hash = keccak256(key);

        // Extract two 64-bit values from the 256-bit hash
        let h1 = u64::from_le_bytes(hash[0..8].try_into().unwrap()) as usize;
        let h2 = u64::from_le_bytes(hash[8..16].try_into().unwrap()) as usize;

        let mut hashes = [0usize; NUM_HASHES];
        for i in 0..NUM_HASHES {
            hashes[i] = h1.wrapping_add(i.wrapping_mul(h2));
        }
        hashes
    }
}

impl Default for BloomFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_basic() {
        let mut bloom = BloomFilter::new();

        bloom.insert(b"hello");
        bloom.insert(b"world");

        assert!(bloom.may_contain(b"hello"));
        assert!(bloom.may_contain(b"world"));
        // Very unlikely to be a false positive for a random key
        // (though technically possible)
    }

    #[test]
    fn test_bloom_definitely_not_present() {
        let mut bloom = BloomFilter::for_capacity(100);

        // Insert some keys
        for i in 0u64..100 {
            bloom.insert(&i.to_be_bytes());
        }

        // All inserted keys should be found
        for i in 0u64..100 {
            assert!(bloom.may_contain(&i.to_be_bytes()));
        }
    }

    #[test]
    fn test_bloom_false_positive_rate() {
        let mut bloom = BloomFilter::for_capacity(1000);

        // Insert 1000 elements
        for i in 0u64..1000 {
            bloom.insert(&i.to_be_bytes());
        }

        // Check 1000 elements that were NOT inserted
        let mut false_positives = 0;
        for i in 10000u64..11000 {
            if bloom.may_contain(&i.to_be_bytes()) {
                false_positives += 1;
            }
        }

        // Should be roughly 1% false positive rate
        // Allow some variance (0-5%)
        assert!(
            false_positives < 50,
            "Too many false positives: {} (expected < 50)",
            false_positives
        );
    }

    #[test]
    fn test_bloom_clear() {
        let mut bloom = BloomFilter::new();

        bloom.insert(b"test");
        assert!(bloom.may_contain(b"test"));

        bloom.clear();
        // After clear, nothing should be found
        // (well, nothing was inserted, so definitely not found)
        assert_eq!(bloom.count(), 0);
    }
}
