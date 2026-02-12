use std::collections::HashMap;

/// Request types for per-type throughput tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestType {
    Headers,
    Bodies,
    AccountRange,
    StorageRange,
    ByteCodes,
    StateTrieNodes,
    StorageTrieNodes,
}

/// Exponential moving average for throughput (items/ms)
#[derive(Debug, Clone)]
pub struct ThroughputEma {
    /// Current EMA value (items/ms)
    pub value: f64,
    /// Smoothing factor
    alpha: f64,
    /// Number of recorded samples
    pub samples: u32,
    /// Sum for cold-start averaging (first N samples)
    cold_start_sum: f64,
}

const COLD_START_SAMPLES: u32 = 3;

impl ThroughputEma {
    pub fn new(alpha: f64) -> Self {
        Self {
            value: 0.0,
            alpha,
            samples: 0,
            cold_start_sum: 0.0,
        }
    }

    pub fn record(&mut self, items_per_ms: f64) {
        self.samples += 1;
        if self.samples <= COLD_START_SAMPLES {
            self.cold_start_sum += items_per_ms;
            self.value = self.cold_start_sum / self.samples as f64;
        } else {
            self.value = self.alpha * items_per_ms + (1.0 - self.alpha) * self.value;
        }
    }
}

impl Default for ThroughputEma {
    fn default() -> Self {
        Self::new(0.2)
    }
}

/// Per-peer metrics for throughput, RTT, and per-type failure tracking
#[derive(Debug, Clone)]
pub struct PeerMetrics {
    /// Per-type throughput EMA (items/ms)
    pub throughput: HashMap<RequestType, ThroughputEma>,
    /// Legacy score for backward compatibility
    pub score: i64,
    /// RTT exponential moving average (ms)
    pub rtt_ema_ms: f64,
    /// RTT sample count
    pub rtt_samples: u32,
    /// Total successful requests
    pub successes: u64,
    /// Total failed requests
    pub failures: u64,
    /// Per-type success counts
    pub successes_by_type: HashMap<RequestType, u64>,
    /// Per-type failure counts
    pub failures_by_type: HashMap<RequestType, u64>,
}

const RTT_EMA_ALPHA: f64 = 0.2;

impl PeerMetrics {
    pub fn record_throughput(
        &mut self,
        request_type: RequestType,
        items_per_ms: f64,
        rtt_ms: f64,
    ) {
        self.throughput
            .entry(request_type)
            .or_default()
            .record(items_per_ms);
        self.rtt_samples += 1;
        if self.rtt_samples <= 3 {
            self.rtt_ema_ms = (self.rtt_ema_ms * (self.rtt_samples - 1) as f64 + rtt_ms)
                / self.rtt_samples as f64;
        } else {
            self.rtt_ema_ms = RTT_EMA_ALPHA * rtt_ms + (1.0 - RTT_EMA_ALPHA) * self.rtt_ema_ms;
        }
    }

    pub fn throughput_for(&self, request_type: RequestType) -> f64 {
        self.throughput
            .get(&request_type)
            .map(|t| t.value)
            .unwrap_or(0.0)
    }

    pub fn record_success_for(&mut self, request_type: RequestType) {
        self.successes += 1;
        *self.successes_by_type.entry(request_type).or_insert(0) += 1;
    }

    pub fn record_failure_for(&mut self, request_type: RequestType) {
        self.failures += 1;
        *self.failures_by_type.entry(request_type).or_insert(0) += 1;
    }

    /// Returns true if this peer has >50% failure rate for a request type after 5+ attempts
    pub fn is_weak_for(&self, request_type: RequestType) -> bool {
        let failures = self.failures_by_type.get(&request_type).copied().unwrap_or(0);
        let successes = self
            .successes_by_type
            .get(&request_type)
            .copied()
            .unwrap_or(0);
        let total = failures + successes;
        total >= 5 && (failures as f64 / total as f64) > 0.5
    }

    /// Adaptive timeout: 2x RTT EMA, clamped [2s, 30s]. Returns None if cold (< 3 samples).
    pub fn adaptive_timeout_ms(&self) -> Option<f64> {
        if self.rtt_samples < 3 {
            return None;
        }
        Some((self.rtt_ema_ms * 2.0).clamp(2_000.0, 30_000.0))
    }

    /// Adaptive request size: throughput x target_rtt (500ms), clamped [min, max]. Returns None if cold.
    pub fn adaptive_request_size(
        &self,
        request_type: RequestType,
        min_items: usize,
        max_items: usize,
    ) -> Option<usize> {
        let throughput = self.throughput_for(request_type);
        if throughput == 0.0 {
            return None;
        }
        let target_rtt_ms = 500.0;
        let estimated = (throughput * target_rtt_ms) as usize;
        Some(estimated.clamp(min_items, max_items))
    }
}

impl Default for PeerMetrics {
    fn default() -> Self {
        Self {
            throughput: HashMap::new(),
            score: 0,
            rtt_ema_ms: 0.0,
            rtt_samples: 0,
            successes: 0,
            failures: 0,
            successes_by_type: HashMap::new(),
            failures_by_type: HashMap::new(),
        }
    }
}
