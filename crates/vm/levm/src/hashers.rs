//! zkVM-tuned hashers for the hot in-memory maps/sets.
//!
//! On ZisK (riscv64ima, no unaligned scalar loads) the default fxhash spends
//! most of its cost simply *reading* the 20/32 key bytes as `lbu` chains
//! (~3 instr/byte), not on its multiplies — ~82 instr for an `Address`, ~128 for
//! an `H256`, paid on every warmth check / cache probe (4 per warm SLOAD). Since
//! these keys are already uniformly distributed (keccak outputs or high-entropy
//! addresses), we don't need fxhash's mixing at all: read 8 bytes of the key
//! directly as the bucket hash (~23 instr, zero multiplies).
//!
//! Correctness note: a hashmap's *correctness* never depends on hash quality
//! (equality is exact), only its *performance*. A poor choice here shows up as a
//! slowdown (collision chains), never as a wrong result — so the state-root gate
//! plus a slowdown in the bench are the safety net. The endianness choices below
//! keep collisions low for the real key distributions.

use core::hash::{BuildHasherDefault, Hasher};
use std::collections::{HashMap, HashSet};

/// Truncating hasher for keys that are uniform across their **trailing** bytes:
/// `Address` (keccak-derived; vanity contracts only zero *leading* bytes) and
/// keccak-derived `H256` keys (code hashes, hashed addresses/roots). Uses the
/// last 8 bytes of the key as the hash.
#[derive(Default)]
pub struct TruncHasher(u64);

impl Hasher for TruncHasher {
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        let b = match bytes.last_chunk::<8>() {
            Some(tail) => *tail,
            // Keys shorter than the window (never on the hot paths) hash on
            // what they have.
            None => {
                let mut short = [0u8; 8];
                for (dst, src) in short.iter_mut().zip(bytes) {
                    *dst = *src;
                }
                short
            }
        };
        self.0 = u64::from_le_bytes(b);
    }
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.0
    }
}

/// Hasher for EVM storage-slot `H256` keys, which are **big-endian** and a mix of
/// small integers (slots 0,1,2,…) and keccak-derived mapping slots. Reading the
/// last 8 bytes big-endian maps small ints to themselves (distinct buckets)
/// while keeping keccak keys uniform; XOR-ing the first-8 little-endian folds in
/// the keccak entropy of mapping slots. The `h ^ (h << 57)` finisher spreads
/// low-byte entropy into hashbrown's top-7 control bits so sequential small
/// slots don't share a control byte.
#[derive(Default)]
pub struct SlotHasher(u64);

impl Hasher for SlotHasher {
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        let (lo, hi) = match (bytes.first_chunk::<8>(), bytes.last_chunk::<8>()) {
            (Some(head), Some(tail)) => (*head, *tail),
            // Keys shorter than the window (never on the hot paths) hash on
            // what they have, in both windows.
            _ => {
                let mut short = [0u8; 8];
                for (dst, src) in short.iter_mut().zip(bytes) {
                    *dst = *src;
                }
                (short, short)
            }
        };
        self.0 = u64::from_le_bytes(lo) ^ u64::from_be_bytes(hi);
    }
    #[inline(always)]
    fn finish(&self) -> u64 {
        let h = self.0;
        h ^ (h << 57)
    }
}

pub type TruncBuild = BuildHasherDefault<TruncHasher>;
pub type SlotBuild = BuildHasherDefault<SlotHasher>;

/// `HashMap`/`HashSet` keyed by `Address` or keccak-`H256` (trailing-uniform).
pub type TruncMap<K, V> = HashMap<K, V, TruncBuild>;
pub type TruncSet<K> = HashSet<K, TruncBuild>;
/// `HashMap`/`HashSet` keyed by an EVM storage-slot `H256`.
pub type SlotMap<K, V> = HashMap<K, V, SlotBuild>;
pub type SlotSet<K> = HashSet<K, SlotBuild>;

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;

    fn trunc(k: &[u8]) -> u64 {
        let mut h = TruncHasher::default();
        h.write(k);
        h.finish()
    }
    fn slot(k: &H256) -> u64 {
        let mut h = SlotHasher::default();
        h.write(&k.0);
        h.finish()
    }

    #[test]
    fn trunc_distinguishes_trailing_bytes() {
        // Vanity addresses share leading zeros; must differ on the tail.
        let a = [0u8; 20];
        let mut b = [0u8; 20];
        b[19] = 1;
        let mut c = [0u8; 20];
        c[12] = 0xAB; // within the last-8 window
        assert_ne!(trunc(&a), trunc(&b));
        assert_ne!(trunc(&a), trunc(&c));
    }

    #[test]
    fn slot_small_ints_are_distinct() {
        // Big-endian H256 of small integers 0..64 must all hash distinctly.
        let mut seen = std::collections::HashSet::new();
        for i in 0u64..64 {
            let key = H256::from_low_u64_be(i);
            assert!(seen.insert(slot(&key)), "collision at small slot {i}");
        }
    }

    #[test]
    fn slot_map_roundtrips() {
        let mut m: SlotMap<H256, u64> = SlotMap::default();
        for i in 0u64..1000 {
            m.insert(H256::from_low_u64_be(i), i);
        }
        for i in 0u64..1000 {
            assert_eq!(m.get(&H256::from_low_u64_be(i)), Some(&i));
        }
    }
}
