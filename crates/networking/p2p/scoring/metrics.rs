//! Metric tracking utilities for peer scoring.
//!
//! Provides EWMA, latency percentiles, and throughput tracking.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default alpha for EWMA (weight for new values).
/// A value of 0.2 means new samples contribute 20% to the average.
const DEFAULT_EWMA_ALPHA: f64 = 0.2;

/// Default size for the latency sample buffer.
const DEFAULT_LATENCY_BUFFER_SIZE: usize = 100;

/// Exponentially Weighted Moving Average (EWMA) calculator.
///
/// EWMA smooths out noisy data while giving more weight to recent observations.
/// The formula is: EWMA_new = alpha * new_value + (1 - alpha) * EWMA_old
///
/// Higher alpha values make the average more responsive to recent changes
/// but also more sensitive to noise.
#[derive(Debug, Clone)]
pub struct EWMA {
    /// The smoothing factor (0 < alpha <= 1)
    alpha: f64,
    /// The current EWMA value
    value: Option<f64>,
}

impl Default for EWMA {
    fn default() -> Self {
        Self::new(DEFAULT_EWMA_ALPHA)
    }
}

impl EWMA {
    /// Creates a new EWMA with the given alpha value.
    ///
    /// # Panics
    ///
    /// Panics if alpha is not in the range (0, 1].
    pub fn new(alpha: f64) -> Self {
        assert!(
            (0.0..=1.0).contains(&alpha) && alpha > 0.0,
            "alpha must be in range (0, 1]"
        );
        Self { alpha, value: None }
    }

    /// Updates the EWMA with a new sample value.
    pub fn update(&mut self, sample: f64) {
        self.value = Some(match self.value {
            Some(current) => self.alpha * sample + (1.0 - self.alpha) * current,
            None => sample,
        });
    }

    /// Returns the current EWMA value, or the default if no samples yet.
    pub fn value(&self) -> Option<f64> {
        self.value
    }

    /// Returns the current EWMA value, or the provided default if no samples.
    pub fn value_or(&self, default: f64) -> f64 {
        self.value.unwrap_or(default)
    }

    /// Resets the EWMA to its initial state.
    pub fn reset(&mut self) {
        self.value = None;
    }

    /// Returns true if the EWMA has at least one sample.
    pub fn has_samples(&self) -> bool {
        self.value.is_some()
    }
}

/// Tracks latency samples and calculates percentiles.
///
/// Maintains a circular buffer of recent latency samples for
/// calculating p50, p95, and p99 percentiles.
#[derive(Debug, Clone)]
pub struct LatencyTracker {
    /// Circular buffer of latency samples in milliseconds
    samples: VecDeque<u64>,
    /// Maximum number of samples to keep
    max_samples: usize,
    /// EWMA for smoothed latency
    ewma: EWMA,
}

impl Default for LatencyTracker {
    fn default() -> Self {
        Self::new(DEFAULT_LATENCY_BUFFER_SIZE)
    }
}

impl LatencyTracker {
    /// Creates a new latency tracker with the given buffer size.
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_samples),
            max_samples,
            ewma: EWMA::default(),
        }
    }

    /// Records a new latency sample.
    pub fn record(&mut self, latency: Duration) {
        let millis = latency.as_millis() as u64;

        // Add to circular buffer
        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(millis);

        // Update EWMA
        self.ewma.update(millis as f64);
    }

    /// Returns the EWMA latency in milliseconds.
    pub fn ewma_ms(&self) -> Option<f64> {
        self.ewma.value()
    }

    /// Returns the EWMA latency as a Duration.
    pub fn ewma(&self) -> Option<Duration> {
        self.ewma.value().map(|ms| Duration::from_millis(ms as u64))
    }

    /// Returns the specified percentile latency.
    ///
    /// Percentile should be in range [0, 100].
    pub fn percentile(&self, p: u8) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }

        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();

        let index = ((p as f64 / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        Some(Duration::from_millis(sorted[index]))
    }

    /// Returns the p50 (median) latency.
    pub fn p50(&self) -> Option<Duration> {
        self.percentile(50)
    }

    /// Returns the p95 latency.
    pub fn p95(&self) -> Option<Duration> {
        self.percentile(95)
    }

    /// Returns the p99 latency.
    pub fn p99(&self) -> Option<Duration> {
        self.percentile(99)
    }

    /// Returns the number of samples recorded.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Resets all latency data.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.ewma.reset();
    }
}

/// Tracks throughput (bytes per second).
///
/// Uses EWMA for smoothed throughput calculation.
#[derive(Debug, Clone)]
pub struct ThroughputTracker {
    /// EWMA of throughput in bytes per second
    ewma: EWMA,
    /// Last sample timestamp for rate calculation
    last_sample: Option<Instant>,
    /// Accumulated bytes since last rate calculation
    pending_bytes: u64,
}

impl Default for ThroughputTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ThroughputTracker {
    /// Creates a new throughput tracker.
    pub fn new() -> Self {
        Self {
            ewma: EWMA::new(0.3), // Slightly more responsive for throughput
            last_sample: None,
            pending_bytes: 0,
        }
    }

    /// Records a data transfer.
    pub fn record(&mut self, bytes: u64, duration: Duration) {
        if duration.as_millis() > 0 {
            let bytes_per_sec = (bytes as f64 / duration.as_secs_f64()).max(0.0);
            self.ewma.update(bytes_per_sec);
        }
    }

    /// Returns the smoothed throughput in bytes per second.
    pub fn bytes_per_sec(&self) -> Option<f64> {
        self.ewma.value()
    }

    /// Returns the smoothed throughput in MB per second.
    pub fn mb_per_sec(&self) -> Option<f64> {
        self.ewma.value().map(|bps| bps / (1024.0 * 1024.0))
    }

    /// Resets all throughput data.
    pub fn reset(&mut self) {
        self.ewma.reset();
        self.last_sample = None;
        self.pending_bytes = 0;
    }
}

/// Combined metrics for a specific request type.
///
/// Tracks success/failure counts, latency, and throughput.
#[derive(Debug, Clone)]
pub struct RequestTypeMetrics {
    /// Total successful requests
    pub successes: u64,
    /// Total failed requests
    pub failures: u64,
    /// Latency tracking
    pub latency: LatencyTracker,
    /// Throughput tracking (for data-heavy requests)
    pub throughput: ThroughputTracker,
    /// Timestamp of first request (for reliability calculation)
    pub first_request_at: Option<Instant>,
    /// Timestamp of last request
    pub last_request_at: Option<Instant>,
}

impl Default for RequestTypeMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestTypeMetrics {
    /// Creates new request type metrics.
    pub fn new() -> Self {
        Self {
            successes: 0,
            failures: 0,
            latency: LatencyTracker::default(),
            throughput: ThroughputTracker::default(),
            first_request_at: None,
            last_request_at: None,
        }
    }

    /// Records a successful request.
    pub fn record_success(&mut self, latency: Duration, bytes: Option<u64>) {
        let now = Instant::now();

        self.successes += 1;
        self.latency.record(latency);

        if let Some(bytes) = bytes {
            self.throughput.record(bytes, latency);
        }

        if self.first_request_at.is_none() {
            self.first_request_at = Some(now);
        }
        self.last_request_at = Some(now);
    }

    /// Records a failed request.
    pub fn record_failure(&mut self) {
        let now = Instant::now();

        self.failures += 1;

        if self.first_request_at.is_none() {
            self.first_request_at = Some(now);
        }
        self.last_request_at = Some(now);
    }

    /// Returns the total number of requests.
    pub fn total_requests(&self) -> u64 {
        self.successes + self.failures
    }

    /// Returns the success rate (0.0 - 1.0).
    pub fn success_rate(&self) -> f64 {
        let total = self.total_requests();
        if total == 0 {
            return 0.5; // Neutral for no data
        }
        self.successes as f64 / total as f64
    }

    /// Resets all metrics.
    pub fn reset(&mut self) {
        self.successes = 0;
        self.failures = 0;
        self.latency.reset();
        self.throughput.reset();
        self.first_request_at = None;
        self.last_request_at = None;
    }
}
