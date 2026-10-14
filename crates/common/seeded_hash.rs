//! A seeded hasher for maps whose keys are chosen by contract code or by the block producer.
//!
//! rustc-hash 2.x is unseeded and hashes a key of 17 to 32 bytes as
//! `multiply_mix(SEED2 ^ word(bytes[len - 16..len - 8]), ..) ^ len`, where `SEED2` is a
//! public constant and `multiply_mix(0, _) == 0`. Keys carrying `SEED2` in that word all hash
//! to `len` and share one hashbrown bucket, so storage slots, touched addresses or calldata
//! crafted that way turn every lookup into a scan, at flat opcode gas. [`SeededState`] hashes
//! with foldhash instead, seeded per process and per map, with one twist: a 32- or 20-byte
//! key (`H256`, `U256`, `Address`) is fed to foldhash as words rather than as a byte string.
//! foldhash's byte path spends four folded multiplies on 32 bytes where its integer path
//! spends two, which shows on storage-heavy code; the word path resists the same attacks,
//! since every multiply still mixes in a seed the key does not know.
//!
//! Maps keyed by keccak outputs (code hashes, transaction hashes, created contract addresses,
//! recovered senders) can stay on FxHash: nobody can steer those keys into a bucket.

use std::hash::{BuildHasher, Hasher};

/// A `std::collections::HashMap` hashed with [`SeededState`].
pub type SeededHashMap<K, V> = std::collections::HashMap<K, V, SeededState>;
/// A `std::collections::HashSet` hashed with [`SeededState`].
pub type SeededHashSet<K> = std::collections::HashSet<K, SeededState>;

/// A foldhash `RandomState`, seeded per process and per map, whose hasher takes fixed-size
/// keys through foldhash's integer path. See the module docs.
#[derive(Clone, Debug, Default)]
pub struct SeededState(foldhash::fast::RandomState);

impl BuildHasher for SeededState {
    type Hasher = SeededHasher;

    #[inline]
    fn build_hasher(&self) -> SeededHasher {
        SeededHasher(self.0.build_hasher())
    }
}

/// The hasher built by [`SeededState`].
pub struct SeededHasher(foldhash::fast::FoldHasher<'static>);

impl Hasher for SeededHasher {
    /// `H256` and `Address` hash as one 32- or 20-byte write, and `U256` as a length prefix
    /// plus one 32-byte write. Those go through `write_u128`, which folds 16 bytes per
    /// multiply; every other length takes foldhash's byte path.
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        match bytes.len() {
            32 => {
                if let Some((lo, rest)) = bytes.split_first_chunk::<16>()
                    && let Some(hi) = rest.first_chunk::<16>()
                {
                    self.0.write_u128(u128::from_ne_bytes(*lo));
                    self.0.write_u128(u128::from_ne_bytes(*hi));
                    return;
                }
            }
            20 => {
                if let Some((lo, rest)) = bytes.split_first_chunk::<16>()
                    && let Some(hi) = rest.first_chunk::<4>()
                {
                    self.0.write_u128(u128::from_ne_bytes(*lo));
                    self.0.write_u32(u32::from_ne_bytes(*hi));
                    return;
                }
            }
            _ => {}
        }
        self.0.write(bytes);
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.0.write_u8(i);
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.0.write_u16(i);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0.write_u32(i);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0.write_u64(i);
    }

    #[inline]
    fn write_u128(&mut self, i: u128) {
        self.0.write_u128(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.0.write_usize(i);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Address, H256, U256};
    use std::{collections::HashSet, hash::Hash};

    /// `SEED2` in rustc-hash 2.x: the word XORed into a 17- to 32-byte key right before its
    /// final multiply.
    const ZEROING_WORD: u64 = 0x13198a2e03707344;

    fn distinct_hashes<K: Hash>(hasher: &impl BuildHasher, keys: &[K]) -> usize {
        keys.iter()
            .map(|key| hasher.hash_one(key))
            .collect::<HashSet<_>>()
            .len()
    }

    /// Checks that `keys` share one FxHash, so they are the attack shape, and that the seeded
    /// state gives each of them its own hash.
    fn assert_hashed_apart<K: Hash>(keys: &[K]) {
        assert_eq!(distinct_hashes(&rustc_hash::FxBuildHasher, keys), 1);
        assert_eq!(distinct_hashes(&SeededState::default(), keys), keys.len());
    }

    #[test]
    fn hashes_colliding_h256_keys_apart() {
        let keys: Vec<H256> = (0..4096u64)
            .map(|i| H256::from(U256([i, 0, ZEROING_WORD, 0]).to_little_endian()))
            .collect();
        assert_hashed_apart(&keys);
    }

    #[test]
    fn hashes_colliding_u256_keys_apart() {
        let keys: Vec<U256> = (0..4096u64)
            .map(|i| U256([i, 0, ZEROING_WORD, 0]))
            .collect();
        assert_hashed_apart(&keys);
    }

    #[test]
    fn hashes_colliding_addresses_apart() {
        let keys: Vec<Address> = (0..4096u64)
            .map(|i| {
                let mut bytes = [0u8; 20];
                for (byte, value) in bytes.iter_mut().skip(4).zip(ZEROING_WORD.to_le_bytes()) {
                    *byte = value;
                }
                for (byte, value) in bytes.iter_mut().skip(12).zip(i.to_le_bytes()) {
                    *byte = value;
                }
                Address::from(bytes)
            })
            .collect();
        assert_hashed_apart(&keys);
    }

    #[test]
    fn hashes_colliding_byte_strings_apart() {
        // 32-byte calldata takes the word path; 40 bytes takes foldhash's byte path.
        let short: Vec<Vec<u8>> = (0..4096u64)
            .map(|i| U256([i, 0, ZEROING_WORD, 0]).to_little_endian().to_vec())
            .collect();
        assert_hashed_apart(&short);
        let long: Vec<Vec<u8>> = (0..4096u64)
            .map(|i| {
                let mut bytes = [0u8; 40];
                for (byte, value) in bytes.iter_mut().zip(i.to_le_bytes()) {
                    *byte = value;
                }
                bytes.to_vec()
            })
            .collect();
        // FxHash runs its bulk loop twice on 40 bytes, so the zeroing word is the state the
        // first round leaves rather than SEED2; only the seeded side is asserted here.
        assert_eq!(distinct_hashes(&SeededState::default(), &long), long.len());
    }

    #[test]
    fn equal_keys_hash_equal_on_both_paths() {
        let state = SeededState::default();
        let key = H256::from_low_u64_be(7);
        assert_eq!(state.hash_one(key), state.hash_one(key));
        let bytes = vec![1u8; 40];
        assert_eq!(state.hash_one(&bytes), state.hash_one(&bytes));
    }
}
