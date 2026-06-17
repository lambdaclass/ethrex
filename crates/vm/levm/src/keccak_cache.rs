//! Host-only result cache for the `KECCAK256` opcode (0x20).
//!
//! The opcode fires extremely often on a tiny, highly-repeated set of inputs:
//! measured on mainnet (blocks 25087279–25087684, 114,692 txs) it ran **1.79M
//! times on only 294K distinct inputs** (mean 6.1 reuses each), dominated by
//! Solidity mapping-slot derivation `keccak(key‖slot)` whose per-call input is
//! **p50 = p90 = 64 B**. An infinite cache would hit 83.6% of calls.
//!
//! `keccak256` is a pure function, so returning a previously-computed hash for
//! the same bytes is fully transparent: the result is identical to recomputing
//! it. We exploit that to skip the (hand-written-assembly) permutation on a hit.
//! A microbench on this hardware put `keccak256(64B)` at ~141 ns vs ~7 ns for a
//! probe + exact compare — ~20× cheaper, so even a modest hit rate is a win.
//!
//! # Design
//!
//! - **Direct-mapped**, `[SLOTS]` fixed slots indexed by a non-crypto hash of
//!   the input. Each slot stores the **full input bytes** and the result; on a
//!   probe we compare the stored bytes *exactly*. A hash collision therefore
//!   only causes two inputs to share (and mutually evict) a slot — it can never
//!   return a wrong hash. There is **no fingerprint key** (a fingerprint
//!   collision would be a consensus split).
//! - **Bounded**: fixed-size, no growth, no eviction bookkeeping, no allocation
//!   after construction. Cheapest possible policy for a hot path; the workload
//!   is recency-bursty so a bookkeeping-free policy keeps ~all of LRU's benefit.
//! - **Inputs longer than `CAP` bypass the cache** and hash directly — the fat
//!   tail (CREATE init-code, large memory blobs) is one-shot and doesn't repeat,
//!   and the cap lets us use a fixed inline key with no per-entry heap alloc.
//! - **Thread-local**: the `KECCAK256` opcode only runs on execution threads.
//!   A thread-local needs no lock; under parallel block execution it becomes
//!   per-worker (independent, still transparent, somewhat lower hit rate).
//!
//! # Guest / zkVM
//!
//! This module is compiled **only for the host** (`not(target_arch =
//! "riscv64")`). The zkVM guest runs on riscv64 and must keep the opcode on its
//! direct, provable path — the cache must not change guest behavior, witness
//! output, or proving cost. See [`super::opcode_handlers::keccak`].

use std::cell::RefCell;
use std::hash::{Hash, Hasher};

use ethrex_common::{U256, utils::u256_from_big_endian};
use ethrex_crypto::Crypto;
use rustc_hash::FxHasher;

/// Max input length we cache. 64 B captures the dominant mapping-slot case;
/// 128 B keeps 97.6% of all reuse (vs 95.5% at 64 B) while still dropping the
/// 94 KB tail. Inputs above this hash directly.
const CAP: usize = 128;

/// Number of direct-mapped slots (power of two so the index is a mask). 32K is
/// ~95% of the infinite-cache hit ceiling; diminishing returns are smooth.
const SLOTS: usize = 1 << 15;
/// `SLOTS - 1` as a `u64`, to mask the 64-bit hash without a narrowing cast.
/// Kept in sync with `SLOTS` by [`tests::mask_matches_slots`].
const MASK: u64 = (1 << 15) - 1;

thread_local! {
    static CACHE: RefCell<KeccakCache> = RefCell::new(KeccakCache::new());
}

/// Hash `bytes` to a `U256`, consulting the thread-local result cache.
///
/// `bytes` must be non-empty (`KECCAK256` handles the empty input via a
/// constant before reaching here). On a hit the stored result is returned; on a
/// miss `crypto.keccak256` is computed, inserted, and returned.
#[inline]
pub(crate) fn get_or_compute(bytes: &[u8], crypto: &dyn Crypto) -> U256 {
    // Skip the thread-local entirely for inputs we never cache.
    if bytes.len() > CAP {
        return u256_from_big_endian(&crypto.keccak256(bytes));
    }
    CACHE.with(|cache| cache.borrow_mut().get_or_compute(bytes, crypto))
}

#[inline(always)]
fn slot_index(bytes: &[u8]) -> usize {
    // `<[u8]>::hash` bulk-feeds the bytes to `FxHasher::write` (8 bytes/step) and
    // length-prefixes, so distinct-length inputs don't alias on content.
    let mut hasher = FxHasher::default();
    bytes.hash(&mut hasher);
    // The masked value is `< SLOTS`, so it always fits in `usize` (the fallback
    // is unreachable) and is always a valid index into the `SLOTS`-long arrays.
    usize::try_from(hasher.finish() & MASK).unwrap_or(0)
}

/// Structure-of-arrays direct-mapped cache. `lens[i] == 0` marks an empty slot
/// (cached inputs are always `1..=CAP`), so probing a miss usually only touches
/// the small `lens` array and never the 4 MB `keys` array.
struct KeccakCache {
    /// Stored input bytes per slot (only `[..lens[i]]` is meaningful).
    keys: Vec<[u8; CAP]>,
    /// Length of the stored input, `0` when the slot is empty.
    lens: Vec<usize>,
    /// `keccak256` of the stored input, as a `U256` ready to push.
    vals: Vec<U256>,
}

impl KeccakCache {
    fn new() -> Self {
        Self {
            keys: vec![[0u8; CAP]; SLOTS],
            lens: vec![0usize; SLOTS],
            vals: vec![U256::zero(); SLOTS],
        }
    }

    // `idx` comes from `slot_index`, which masks to `< SLOTS`; every array here
    // is exactly `SLOTS` long, so all indexing is in bounds. `[..len]` is in
    // bounds because the caller guarantees `1 <= len <= CAP`.
    #[expect(
        clippy::indexing_slicing,
        reason = "idx < SLOTS == array len; len <= CAP == key len"
    )]
    #[inline]
    fn get_or_compute(&mut self, bytes: &[u8], crypto: &dyn Crypto) -> U256 {
        let len = bytes.len();
        let idx = slot_index(bytes);
        // Hit: same length and exact byte match. `len >= 1` here, so a `0`
        // (empty) slot never matches.
        if self.lens[idx] == len && self.keys[idx][..len] == *bytes {
            return self.vals[idx];
        }
        // Miss: compute, then claim the slot (overwriting any prior occupant).
        let value = u256_from_big_endian(&crypto.keccak256(bytes));
        self.keys[idx][..len].copy_from_slice(bytes);
        self.lens[idx] = len;
        self.vals[idx] = value;
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_crypto::NativeCrypto;

    fn reference(bytes: &[u8]) -> U256 {
        u256_from_big_endian(&NativeCrypto.keccak256(bytes))
    }

    #[test]
    fn mask_matches_slots() {
        // The two consts must stay in lockstep: MASK == SLOTS - 1.
        assert_eq!(usize::try_from(MASK).unwrap_or(0) + 1, SLOTS);
    }

    #[test]
    fn matches_reference_across_sizes() {
        let crypto = NativeCrypto;
        // Cover the boundary (CAP, CAP+1) and the dominant 64 B case.
        for len in [1usize, 31, 32, 33, 63, 64, 65, 127, CAP, CAP + 1, 1024] {
            let input: Vec<u8> = (0..len)
                .map(|i| u8::try_from((i * 7 + 1) % 256).unwrap_or(0))
                .collect();
            let got = get_or_compute(&input, &crypto);
            assert_eq!(got, reference(&input), "len {len} first call");
            // Second call (likely a hit) must return the identical value.
            assert_eq!(
                got,
                get_or_compute(&input, &crypto),
                "len {len} second call"
            );
        }
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "fixed 64-byte scratch buffers, slice index 4 < 64"
    )]
    fn collision_does_not_return_wrong_hash() {
        // Force two distinct inputs into the same slot and check neither ever
        // returns the other's hash — the exact-compare guards against it.
        let mut cache = KeccakCache::new();
        let crypto = NativeCrypto;

        // Find two different inputs that map to the same slot.
        let mut a = vec![0u8; 64];
        let mut b = vec![0u8; 64];
        let mut found = false;
        'outer: for i in 0u32..100_000 {
            a[..4].copy_from_slice(&i.to_le_bytes());
            let ia = slot_index(&a);
            for j in (i + 1)..(i + 4000) {
                b[..4].copy_from_slice(&j.to_le_bytes());
                if slot_index(&b) == ia {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "expected to find a slot collision");
        assert_eq!(slot_index(&a), slot_index(&b));
        assert_ne!(a, b);

        // Insert a, then b (evicts a), then query both: each must be correct.
        assert_eq!(cache.get_or_compute(&a, &crypto), reference(&a));
        assert_eq!(cache.get_or_compute(&b, &crypto), reference(&b)); // evicts a
        assert_eq!(cache.get_or_compute(&a, &crypto), reference(&a)); // recompute, not b's hash
        assert_eq!(cache.get_or_compute(&b, &crypto), reference(&b));
    }

    #[test]
    fn length_disambiguates_prefix() {
        // Inputs where one is a prefix of the other must not alias.
        let mut cache = KeccakCache::new();
        let crypto = NativeCrypto;
        let long = vec![0xABu8; 64];
        let short = vec![0xABu8; 32];
        assert_eq!(cache.get_or_compute(&long, &crypto), reference(&long));
        assert_eq!(cache.get_or_compute(&short, &crypto), reference(&short));
        assert_eq!(cache.get_or_compute(&long, &crypto), reference(&long));
    }
}
