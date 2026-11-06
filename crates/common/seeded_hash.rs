//! A seeded hasher for maps whose keys a block can choose.
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
//! It is foldhash's quality variant, which ends with one more folded multiply than the fast
//! one. hashbrown picks a key's bucket from the low bits of its hash, and with the fast
//! variant those came out of a single multiply: for some seeds, keys that differ only in a
//! short word, such as addresses differing in their last four bytes, crowded a few buckets.
//!
//! A keccak output is no safer as a key than the preimage behind it. Whoever picks the
//! preimage (a CREATE2 salt, the trailing bytes of deployed code) can grind it until the hash
//! lands in a chosen bucket, at about one keccak per bucket of the table for each key. What
//! keeps such a map on FxHash safe is the gas a block pays for each key it adds or looks up.
//! Code hashes are seeded: a contract deployed once can be called again for 100 gas, and every
//! call looks its code up by hash. Created and self-destructed accounts stay on FxHash, since
//! each entry costs the block a CREATE or a SELFDESTRUCT.
//!
//! A seeded map iterates in a different order in every process and every map. Whatever carries
//! that order out of the process (a list another program replays, a report, the offender an
//! error names) has to be sorted, as the prover's host and the guest sort a block's account
//! updates before applying them to the state trie.
//!
//! Inside a zkVM guest foldhash finds no clock and no address randomization, so its seeds
//! follow from the guest program and its input. Crafting keys against a guest then takes
//! replaying it up to the map under attack, which is harder than against FxHash but possible.

use std::hash::{BuildHasher, Hasher};

/// A `std::collections::HashMap` hashed with [`SeededState`].
pub type SeededHashMap<K, V> = std::collections::HashMap<K, V, SeededState>;
/// A `std::collections::HashSet` hashed with [`SeededState`].
pub type SeededHashSet<K> = std::collections::HashSet<K, SeededState>;

/// A foldhash quality `RandomState`, seeded per process and per map, whose hasher takes
/// fixed-size keys through foldhash's integer path. See the module docs.
#[derive(Clone, Debug, Default)]
pub struct SeededState(foldhash::quality::RandomState);

impl BuildHasher for SeededState {
    type Hasher = SeededHasher;

    #[inline]
    fn build_hasher(&self) -> SeededHasher {
        SeededHasher(self.0.build_hasher())
    }
}

/// The hasher built by [`SeededState`].
pub struct SeededHasher(foldhash::quality::FoldHasher<'static>);

impl Hasher for SeededHasher {
    /// `H256` and `Address` hash as one 32- or 20-byte write, and `U256` as a length prefix
    /// plus one 32-byte write. Those go through `write_u128`, which folds 16 bytes per
    /// multiply; every other length takes foldhash's byte path.
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        if let Some((lo, rest)) = bytes.split_first_chunk::<16>() {
            if let Ok(hi) = <[u8; 16]>::try_from(rest) {
                self.0.write_u128(u128::from_ne_bytes(*lo));
                self.0.write_u128(u128::from_ne_bytes(hi));
                return;
            }
            if let Ok(hi) = <[u8; 4]>::try_from(rest) {
                self.0.write_u128(u128::from_ne_bytes(*lo));
                self.0.write_u32(u32::from_ne_bytes(hi));
                return;
            }
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
    use std::hash::Hash;

    /// Keys per test, hashed into as many buckets.
    const KEYS: u64 = 4096;
    /// Hashing 4096 keys uniformly into 4096 buckets fills the fullest one to about 6; a
    /// hasher that ignores the part of the key that varies puts them all in one.
    const MAX_BUCKET_LOAD: usize = 16;
    /// The word FxHash 2.x XORs into a 17- to 32-byte key right before a multiply, which
    /// makes keys carrying it hash alike there. Kept as a hostile key shape; nothing here
    /// relies on rustc-hash still behaving that way.
    const ZEROING_WORD: u64 = 0x13198a2e03707344;

    /// The load of the fullest bucket once `keys` are spread over `KEYS` buckets by the low
    /// bits of their hash, as hashbrown picks a key's first bucket.
    fn fullest_bucket<K: Hash>(keys: impl Iterator<Item = K>) -> usize {
        let state = SeededState::default();
        let mut loads = vec![0usize; KEYS as usize];
        for key in keys {
            let bucket = state.hash_one(key) % KEYS;
            if let Some(load) = loads.get_mut(bucket as usize) {
                *load += 1;
            }
        }
        loads.into_iter().max().unwrap_or_default()
    }

    /// An address whose bytes `at..at + 8` hold `word` and whose bytes 4..12 hold the zeroing
    /// word, so only one region of the key varies.
    fn address_with(at: usize, word: u64) -> Address {
        let mut bytes = [0u8; 20];
        for (byte, value) in bytes.iter_mut().skip(4).zip(ZEROING_WORD.to_le_bytes()) {
            *byte = value;
        }
        for (byte, value) in bytes.iter_mut().skip(at).zip(word.to_le_bytes()) {
            *byte = value;
        }
        Address::from(bytes)
    }

    #[test]
    fn spreads_h256_keys_that_differ_in_either_half() {
        let low = (0..KEYS).map(|i| H256::from(U256([i, 0, ZEROING_WORD, 0]).to_little_endian()));
        assert!(fullest_bucket(low) <= MAX_BUCKET_LOAD);
        let high = (0..KEYS).map(|i| H256::from(U256([0, 0, ZEROING_WORD, i]).to_little_endian()));
        assert!(fullest_bucket(high) <= MAX_BUCKET_LOAD);
    }

    #[test]
    fn spreads_u256_keys_that_differ_in_either_half() {
        let low = (0..KEYS).map(|i| U256([i, 0, ZEROING_WORD, 0]));
        assert!(fullest_bucket(low) <= MAX_BUCKET_LOAD);
        let high = (0..KEYS).map(|i| U256([0, 0, ZEROING_WORD, i]));
        assert!(fullest_bucket(high) <= MAX_BUCKET_LOAD);
    }

    #[test]
    fn spreads_addresses_that_differ_in_either_part() {
        // Bytes 12..20 straddle the 16-byte word and the 4-byte tail; 16..20 is the tail alone.
        assert!(fullest_bucket((0..KEYS).map(|i| address_with(12, i))) <= MAX_BUCKET_LOAD);
        assert!(fullest_bucket((0..KEYS).map(|i| address_with(16, i))) <= MAX_BUCKET_LOAD);
    }

    #[test]
    fn spreads_byte_strings_on_both_paths() {
        // 32-byte calldata takes the word path; 40 bytes takes foldhash's byte path.
        let short = (0..KEYS).map(|i| U256([i, 0, ZEROING_WORD, 0]).to_little_endian().to_vec());
        assert!(fullest_bucket(short) <= MAX_BUCKET_LOAD);
        let long = (0..KEYS).map(|i| {
            let mut bytes = vec![0u8; 40];
            for (byte, value) in bytes.iter_mut().skip(32).zip(i.to_le_bytes()) {
                *byte = value;
            }
            bytes
        });
        assert!(fullest_bucket(long) <= MAX_BUCKET_LOAD);
    }

    #[test]
    fn two_maps_hash_the_same_key_apart() {
        // Each map draws its own seed, so a key crafted against one map's buckets says
        // nothing about another's.
        let key = H256::repeat_byte(0x5a);
        assert_ne!(
            SeededState::default().hash_one(key),
            SeededState::default().hash_one(key)
        );
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
