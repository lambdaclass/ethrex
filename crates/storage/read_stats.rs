//! Opt-in per-column-family read accounting for cold-read cost attribution.
//!
//! Enabled by setting `ETHREX_READ_STATS=1` (or `true`). When enabled,
//! [`record`] accumulates, per column family, the number of backend point reads
//! and the nanoseconds spent in them. [`block_delta_summary`] returns the delta
//! since the previous call, formatted for logging.
//!
//! Block import on the head-following path is sequential (one block through
//! `add_block` at a time), so the delta between two consecutive per-block log
//! calls approximates a single block's reads — including the concurrent warmer
//! and merkleizer reads, which are part of that block's cost. The split by
//! column family separates exec-phase flat-KV *value* reads
//! (`account_flatkeyvalue`/`storage_flatkeyvalue`) from commit-phase *trie-node*
//! reads (`account_trie_nodes`/`storage_trie_nodes`) — the question Step-0 of
//! the XEN optimization plan must settle. The delta is only meaningful for
//! sequential single-block import, NOT parallel batch sync.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};

const N: usize = 5;
const NAMES: [&str; N] = ["acct_fkv", "stor_fkv", "acct_trie", "stor_trie", "other"];

static COUNTS: [AtomicU64; N] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static NANOS: [AtomicU64; N] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static LAST_COUNTS: [AtomicU64; N] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static LAST_NANOS: [AtomicU64; N] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// Whether read accounting is enabled (read once from `ETHREX_READ_STATS`).
#[inline]
pub fn is_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        matches!(
            std::env::var("ETHREX_READ_STATS").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        )
    })
}

#[inline]
fn bucket(table: &str) -> usize {
    match table {
        ACCOUNT_FLATKEYVALUE => 0,
        STORAGE_FLATKEYVALUE => 1,
        ACCOUNT_TRIE_NODES => 2,
        STORAGE_TRIE_NODES => 3,
        _ => 4,
    }
}

/// Record a single backend point read of `table` that took `nanos` ns.
#[inline]
pub fn record(table: &str, nanos: u64) {
    let i = bucket(table);
    COUNTS[i].fetch_add(1, Ordering::Relaxed);
    NANOS[i].fetch_add(nanos, Ordering::Relaxed);
}

/// Per-column-family reads since the previous call, formatted for logging.
/// Returns `None` when accounting is disabled.
pub fn block_delta_summary() -> Option<String> {
    if !is_enabled() {
        return None;
    }
    let mut parts = Vec::with_capacity(N);
    let (mut total_reads, mut total_nanos) = (0u64, 0u64);
    for i in 0..N {
        let c = COUNTS[i].load(Ordering::Relaxed);
        let n = NANOS[i].load(Ordering::Relaxed);
        let dc = c.wrapping_sub(LAST_COUNTS[i].swap(c, Ordering::Relaxed));
        let dn = n.wrapping_sub(LAST_NANOS[i].swap(n, Ordering::Relaxed));
        total_reads = total_reads.saturating_add(dc);
        total_nanos = total_nanos.saturating_add(dn);
        parts.push(format!(
            "{}={} ({:.1}ms)",
            NAMES[i],
            dc,
            dn as f64 / 1_000_000.0
        ));
    }
    Some(format!(
        "{}  | total {} reads, {:.1} ms",
        parts.join("  "),
        total_reads,
        total_nanos as f64 / 1_000_000.0
    ))
}
