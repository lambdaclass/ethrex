//! Per-peer, per-data-type throughput tracking via exponential moving average (EMA).
//!
//! Speed measurement is complementary to the existing reputation scoring:
//! reputation catches misbehavior, speed picks among well-behaved peers.
//! Slow peers are deprioritized for new requests, never evicted.

use std::time::{Duration, Instant};

/// The data types for which we track per-peer transfer speed.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum TransferType {
    Headers,
    Bodies,
    AccountRanges,
    StorageRanges,
    Bytecodes,
    StateNodes,
}

impl TransferType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransferType::Headers => "headers",
            TransferType::Bodies => "bodies",
            TransferType::AccountRanges => "account_ranges",
            TransferType::StorageRanges => "storage_ranges",
            TransferType::Bytecodes => "bytecodes",
            TransferType::StateNodes => "state_nodes",
        }
    }
}

impl std::fmt::Display for TransferType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// EMA smoothing factor. With alpha = 0.3, the half-life is ~2 samples
/// (i.e. after ~2-3 new measurements the old value contributes < 50%).
const EMA_ALPHA: f64 = 0.3;

/// Tracks items-per-second EMA for each [`TransferType`].
///
/// "Items" means headers, bodies, trie nodes, etc. — the count of
/// logical objects returned, not raw bytes. This is robust across
/// different message encodings and matches what matters for sync
/// throughput comparisons.
#[derive(Debug, Clone)]
pub struct SpeedTracker {
    /// Items-per-second EMA, one entry per observed TransferType.
    entries: Vec<SpeedEntry>,
}

#[derive(Debug, Clone)]
struct SpeedEntry {
    transfer_type: TransferType,
    ema_ips: f64,
    last_update: Instant,
}

impl Default for SpeedTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeedTracker {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Record a completed response.
    ///
    /// - `item_count`: number of items in the response (headers, bodies, nodes, etc.)
    /// - `elapsed`: wall-clock time from request dispatch to response receipt
    ///
    /// Empty responses (`item_count == 0`) are silently ignored — they happen
    /// legitimately near the chain tip and would distort the EMA.
    /// Timeouts should not call this method; the reputation system handles those.
    pub fn record(&mut self, transfer_type: TransferType, item_count: usize, elapsed: Duration) {
        if item_count == 0 {
            return;
        }
        let secs = elapsed.as_secs_f64().max(0.001);
        let sample_ips = item_count as f64 / secs;

        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.transfer_type == transfer_type)
        {
            entry.ema_ips = EMA_ALPHA * sample_ips + (1.0 - EMA_ALPHA) * entry.ema_ips;
            entry.last_update = Instant::now();
        } else {
            // First sample for this type: seed EMA with the raw sample.
            self.entries.push(SpeedEntry {
                transfer_type,
                ema_ips: sample_ips,
                last_update: Instant::now(),
            });
        }
    }

    /// Returns the current EMA for `transfer_type`, or `None` if no
    /// measurement has been recorded for that type yet.
    pub fn ema(&self, transfer_type: TransferType) -> Option<f64> {
        self.entries
            .iter()
            .find(|e| e.transfer_type == transfer_type)
            .map(|e| e.ema_ips)
    }
}

/// Compute the quantile rank of `value` within `all_values`.
/// Returns a value in `[0.0, 1.0]`.
/// If `all_values` has fewer than 2 entries, returns 0.5 (no ranking possible).
pub fn quantile_rank(value: f64, all_values: &[f64]) -> f64 {
    if all_values.len() < 2 {
        return 0.5;
    }
    let below = all_values.iter().filter(|&&v| v < value).count();
    let equal = all_values
        .iter()
        .filter(|&&v| (v - value).abs() < f64::EPSILON)
        .count();
    // CDF-style: (below + 0.5 * equal) / total
    (below as f64 + 0.5 * equal as f64) / all_values.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ema_converges() {
        let mut tracker = SpeedTracker::new();
        // Feed 10 samples of 1000 items/sec (1000 items in 1 second)
        for _ in 0..10 {
            tracker.record(TransferType::Headers, 1000, Duration::from_secs(1));
        }
        let ema = tracker.ema(TransferType::Headers).unwrap();
        // After 10 samples of 1000 ips, EMA should be within 5% of 1000
        assert!(
            (ema - 1000.0).abs() < 50.0,
            "EMA should converge to ~1000, got {ema}"
        );
    }

    #[test]
    fn empty_response_ignored() {
        let mut tracker = SpeedTracker::new();
        // Feed 5 samples of 1000 items/sec
        for _ in 0..5 {
            tracker.record(TransferType::Headers, 1000, Duration::from_secs(1));
        }
        let ema_before = tracker.ema(TransferType::Headers).unwrap();
        // Empty response should not change EMA
        tracker.record(TransferType::Headers, 0, Duration::from_secs(1));
        let ema_after = tracker.ema(TransferType::Headers).unwrap();
        assert!(
            (ema_before - ema_after).abs() < f64::EPSILON,
            "Empty response should not change EMA"
        );
    }

    #[test]
    fn type_independence() {
        let mut tracker = SpeedTracker::new();
        // Headers at 1000 ips
        for _ in 0..5 {
            tracker.record(TransferType::Headers, 1000, Duration::from_secs(1));
        }
        // Bodies at 5000 ips
        for _ in 0..5 {
            tracker.record(TransferType::Bodies, 5000, Duration::from_secs(1));
        }
        let headers_ema = tracker.ema(TransferType::Headers).unwrap();
        let bodies_ema = tracker.ema(TransferType::Bodies).unwrap();
        assert!(
            (headers_ema - 1000.0).abs() < 50.0,
            "Headers EMA should be ~1000, got {headers_ema}"
        );
        assert!(
            (bodies_ema - 5000.0).abs() < 250.0,
            "Bodies EMA should be ~5000, got {bodies_ema}"
        );
    }

    #[test]
    fn unmeasured_type_returns_none() {
        let tracker = SpeedTracker::new();
        assert!(tracker.ema(TransferType::Headers).is_none());
    }

    #[test]
    fn quantile_rank_basic() {
        let values = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        // 100 is the lowest -> quantile near 0.1
        let q = quantile_rank(100.0, &values);
        assert!(q < 0.2, "lowest value should have low quantile, got {q}");
        // 500 is the highest -> quantile near 0.9
        let q = quantile_rank(500.0, &values);
        assert!(q > 0.8, "highest value should have high quantile, got {q}");
        // 300 is median -> quantile near 0.5
        let q = quantile_rank(300.0, &values);
        assert!(
            (q - 0.5).abs() < 0.15,
            "median value should have ~0.5 quantile, got {q}"
        );
    }

    #[test]
    fn quantile_rank_single_value() {
        let values = vec![100.0];
        assert!((quantile_rank(100.0, &values) - 0.5).abs() < f64::EPSILON);
    }
}
