//! Per-precompile bounded result cache.
//!
//! Replaces the previous `RwLock<FxHashMap<(Address, Bytes), …>>` with a
//! slot-array indexed by precompile-address byte. Each slot owns a
//! `Mutex<LruCache<Bytes, (Bytes, u64)>>` sized per-precompile.
//!
//! Indexing mirrors the dispatcher's existing `PRECOMPILES: [Option<…>; 512]`
//! table in `precompiles.rs`. Identity (0x04) is unconditionally bypassed at
//! the call site, so its slot stays `None`.
//!
//! Inspired by reth #22900 (per-precompile sizing). ethrex stays on the
//! existing `lru` workspace dep rather than introducing `schnellru` —
//! semantics are identical for our use, and the bounded-sizing win is what
//! matters, not the underlying data structure.
//!
//! Lifetime: `PrecompileCache` lives on `CachingDatabase`, which is recreated
//! per block. The cache is therefore per-block, shared between warmer and
//! executor threads. Per-precompile mutexes shard contention so cross-
//! precompile concurrency is lock-free.

use bytes::Bytes;
use ethrex_common::Address;
use lru::LruCache;
use std::borrow::Cow;
use std::num::NonZeroUsize;
use std::sync::Mutex;

/// Address-index space. Matches the dispatcher table in `execute_precompile`,
/// which uses `u16::from_be_bytes([address[18], address[19]])` to route. The
/// table needs room for `0x100` (P256VERIFY) so 512 covers it with margin.
pub const PRECOMPILE_INDEX_SPACE: usize = 512;

// ---------------------------------------------------------------------------
// Per-precompile capacities.
//
// Caps are per-block (not cross-block) but sized to absorb pathological
// per-block call volumes. A `pub const` per precompile makes them trivially
// tunable without rebuilding the cap table.
// ---------------------------------------------------------------------------

/// 0x01 ECRECOVER: every signed-tx-with-data uses it once externally; cached
/// volume per block scales with `tx_count × n_recovers_in_call`. 4K covers
/// the worst observed mainnet block.
pub const ECRECOVER_CAP: usize = 4_000;

/// 0x02 SHA2_256: working set in reth telemetry was 18 — 1K is overkill but
/// cheap. Output is fixed 32 bytes.
pub const SHA2_256_CAP: usize = 1_000;

/// 0x03 RIPEMD_160: same size class as SHA2_256.
pub const RIPEMD_160_CAP: usize = 1_000;

// 0x04 IDENTITY has no cache slot — it is short-circuited at the call site
// (copying calldata is cheaper than the cache lookup).

/// 0x05 MODEXP: heavy in zk-rollup verifier contracts. reth telemetry: 71.6%
/// hit-rate at 10K with working set ≥10K → upsize to 30K.
pub const MODEXP_CAP: usize = 30_000;

/// 0x06 ECADD (BN254): heavy in zk-pairing checks. reth: 70.7% @ 10K → 30K.
pub const ECADD_CAP: usize = 30_000;

/// 0x07 ECMUL (BN254): the most starved precompile in reth's mainnet
/// telemetry (55% hit-rate at 10K). 50K matches the upstream bump.
pub const ECMUL_CAP: usize = 50_000;

/// 0x08 ECPAIRING: pairings are expensive but few unique inputs per block.
pub const ECPAIRING_CAP: usize = 2_000;

/// 0x09 BLAKE2F: low mainnet volume (Eth1 mining / Filecoin bridges).
pub const BLAKE2F_CAP: usize = 2_000;

/// 0x0a KZG_POINT_EVALUATION: ≤6 blobs/block × multiple verifies on 4844
/// blocks. 4K is generous.
pub const POINT_EVALUATION_CAP: usize = 4_000;

/// 0x0b–0x11 BLS12-381 ops: Prague-only; bounded by current usage.
pub const BLS12_G1ADD_CAP: usize = 2_000;
pub const BLS12_G1MSM_CAP: usize = 2_000;
pub const BLS12_G2ADD_CAP: usize = 2_000;
pub const BLS12_G2MSM_CAP: usize = 2_000;
pub const BLS12_PAIRING_CHECK_CAP: usize = 2_000;
pub const BLS12_MAP_FP_TO_G1_CAP: usize = 2_000;
pub const BLS12_MAP_FP2_TO_G2_CAP: usize = 2_000;

/// 0x100 P256VERIFY: Osaka-era; expected modest growth.
pub const P256VERIFY_CAP: usize = 4_000;

/// Returns the capacity for the precompile at the given dispatch index.
/// `0` means "no cache for this address".
const fn cap_for_index(idx: usize) -> usize {
    match idx {
        0x01 => ECRECOVER_CAP,
        0x02 => SHA2_256_CAP,
        0x03 => RIPEMD_160_CAP,
        // 0x04 IDENTITY: no slot.
        0x05 => MODEXP_CAP,
        0x06 => ECADD_CAP,
        0x07 => ECMUL_CAP,
        0x08 => ECPAIRING_CAP,
        0x09 => BLAKE2F_CAP,
        0x0a => POINT_EVALUATION_CAP,
        0x0b => BLS12_G1ADD_CAP,
        0x0c => BLS12_G1MSM_CAP,
        0x0d => BLS12_G2ADD_CAP,
        0x0e => BLS12_G2MSM_CAP,
        0x0f => BLS12_PAIRING_CHECK_CAP,
        0x10 => BLS12_MAP_FP_TO_G1_CAP,
        0x11 => BLS12_MAP_FP2_TO_G2_CAP,
        0x100 => P256VERIFY_CAP,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Calldata normalizers (placeholder).
//
// Future plans (#5 precompile-effective-input, #8 precompile-normalize-input,
// modeled on Nethermind #11309 / #11373) will populate this table to strip
// trailing zero padding and other no-op input bytes before keying the cache,
// boosting hit rates on inputs that differ only by padding.
//
// All slots are `None` today; the cache currently uses raw calldata as the
// lookup key. The hook is declared so the follow-up plans can land without
// touching this module.
// ---------------------------------------------------------------------------

/// Optional per-precompile calldata normalizer. Returning a borrowed slice
/// avoids allocating when no normalization is needed.
pub type Normalizer = fn(&[u8]) -> Cow<'_, [u8]>;

/// Per-precompile normalizer table. Empty for now — see module docs.
pub static NORMALIZERS: [Option<Normalizer>; PRECOMPILE_INDEX_SPACE] =
    [None; PRECOMPILE_INDEX_SPACE];

// ---------------------------------------------------------------------------
// Cache.
// ---------------------------------------------------------------------------

type CacheEntry = (Bytes, u64);
type Slot = Mutex<LruCache<Bytes, CacheEntry>>;

/// Per-block cache of precompile results, sharded by precompile address.
///
/// Each slot owns its own `Mutex<LruCache<Bytes, (Bytes, u64)>>`, so warmer
/// and executor threads contend only when both touch the same precompile
/// concurrently. Cross-precompile access is lock-free.
pub struct PrecompileCache {
    /// `Box<[Option<Slot>]>` (always length `PRECOMPILE_INDEX_SPACE`) keeps
    /// the per-precompile-Mutex array off the stack and out of `CachingDatabase`'s
    /// own size profile.
    slots: Box<[Option<Slot>]>,
}

impl Default for PrecompileCache {
    fn default() -> Self {
        let slots: Box<[Option<Slot>]> = (0..PRECOMPILE_INDEX_SPACE)
            .map(|idx| {
                let cap = cap_for_index(idx);
                if cap == 0 {
                    None
                } else {
                    // Cap is guaranteed non-zero by `cap_for_index`, but
                    // fall back to MIN if a future edit breaks that
                    // invariant — better to under-cache than to panic.
                    let nz = NonZeroUsize::new(cap).unwrap_or(NonZeroUsize::MIN);
                    Some(Mutex::new(LruCache::new(nz)))
                }
            })
            .collect();
        Self { slots }
    }
}

impl PrecompileCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached result for `(address, calldata)` if present.
    /// Touches the LRU recency.
    pub fn get(&self, address: &Address, calldata: &Bytes) -> Option<CacheEntry> {
        let slot = self.slot(address)?;
        let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Lookup borrows `&[u8]` against an `LruCache<Bytes, _>` — no clone of
        // the calldata key on the hot path. Hash matches because both
        // `Bytes::hash` and `<[u8]>::hash` hash the underlying byte slice.
        guard.get(calldata.as_ref()).cloned()
    }

    /// Inserts a new cache entry, evicting LRU if over capacity. `calldata`
    /// is taken by value so the caller's `Bytes` (an Arc-shared clone)
    /// becomes the cache key without an extra byte copy.
    pub fn insert(&self, address: Address, calldata: Bytes, output: Bytes, gas_cost: u64) {
        let Some(slot) = self.slot(&address) else {
            return;
        };
        let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.put(calldata, (output, gas_cost));
    }

    /// Resolves a precompile address to its slot. Returns `None` for
    /// out-of-range addresses or precompiles without a cache (e.g. identity).
    fn slot(&self, address: &Address) -> Option<&Slot> {
        let idx = precompile_index(address)?;
        self.slots.get(idx).and_then(|s| s.as_ref())
    }
}

/// Maps a precompile `Address` to the cache-slot index. Returns `None` if
/// the address is outside the precompile space.
fn precompile_index(address: &Address) -> Option<usize> {
    let bytes = address.as_bytes();
    let prefix = bytes.get(0..18)?;
    if prefix != [0u8; 18] {
        return None;
    }
    let high = bytes.get(18).copied()?;
    let low = bytes.get(19).copied()?;
    Some(usize::from(u16::from_be_bytes([high, low])))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code"
)]
mod tests {
    use super::*;
    use ethrex_common::H160;

    fn addr(byte: u8) -> Address {
        let mut a = [0u8; 20];
        a[19] = byte;
        H160(a)
    }

    #[test]
    fn slot_isolation_across_precompiles() {
        // Same input, different precompile addresses must not collide.
        let cache = PrecompileCache::new();
        let calldata = Bytes::from_static(&[1, 2, 3]);
        cache.insert(
            addr(0x05),
            calldata.clone(),
            Bytes::from_static(b"modexp_out"),
            7,
        );
        cache.insert(
            addr(0x06),
            calldata.clone(),
            Bytes::from_static(b"ecadd_out"),
            11,
        );
        let modexp = cache.get(&addr(0x05), &calldata).unwrap();
        let ecadd = cache.get(&addr(0x06), &calldata).unwrap();
        assert_eq!(modexp, (Bytes::from_static(b"modexp_out"), 7));
        assert_eq!(ecadd, (Bytes::from_static(b"ecadd_out"), 11));
    }

    #[test]
    fn identity_has_no_slot() {
        let cache = PrecompileCache::new();
        // 0x04 is bypassed at the call site; the cache should never persist
        // identity inputs even if `insert` is mistakenly called.
        cache.insert(
            addr(0x04),
            Bytes::from_static(&[1]),
            Bytes::from_static(&[1]),
            1,
        );
        assert!(cache.get(&addr(0x04), &Bytes::from_static(&[1])).is_none());
    }

    #[test]
    fn lookup_borrows_without_cloning_key() {
        // Sanity-check that a `Bytes` key can be looked up via a `&[u8]`
        // borrow form, which is the whole point of dropping `Bytes::clone`
        // from the lookup path.
        let cache = PrecompileCache::new();
        let key = Bytes::from_static(b"ecrecover-input");
        cache.insert(addr(0x01), key, Bytes::from_static(b"output"), 3000);
        // Look up using a freshly-built Bytes (different Arc, same bytes).
        let probe = Bytes::copy_from_slice(b"ecrecover-input");
        let hit = cache.get(&addr(0x01), &probe).unwrap();
        assert_eq!(hit.0, Bytes::from_static(b"output"));
    }

    #[test]
    fn lru_eviction_when_full() {
        // Sanity: stuffing one slot past capacity evicts the oldest entry.
        // Use a small slot (RIPEMD_160 cap = 1_000) and probe the boundary.
        let cache = PrecompileCache::new();
        let pre = addr(0x03);
        for i in 0..=RIPEMD_160_CAP {
            let bytes = Bytes::copy_from_slice(&i.to_le_bytes());
            cache.insert(pre, bytes, Bytes::from_static(b"x"), 1);
        }
        // The very first inserted key must have been evicted.
        let first_key = Bytes::copy_from_slice(&0_usize.to_le_bytes());
        assert!(cache.get(&pre, &first_key).is_none());
        // The most-recent key must still be present.
        let last_key = Bytes::copy_from_slice(&RIPEMD_160_CAP.to_le_bytes());
        assert!(cache.get(&pre, &last_key).is_some());
    }
}
