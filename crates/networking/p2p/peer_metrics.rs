//! Peer performance metrics for adaptive request sizing and parallel peer requests.
//!
//! This module tracks per-peer response times and success rates to:
//! 1. Dynamically adjust request sizes based on peer performance
//! 2. Route requests to faster peers
//! 3. Enable parallel requests to multiple peers for the same range

use ethrex_common::H256;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Default request size in bytes for new/unknown peers
pub const DEFAULT_REQUEST_BYTES: u64 = 512 * 1024; // 512KB
/// Minimum request size in bytes
pub const MIN_REQUEST_BYTES: u64 = 128 * 1024; // 128KB
/// Maximum request size in bytes for fast peers
pub const MAX_REQUEST_BYTES: u64 = 2 * 1024 * 1024; // 2MB

/// Number of recent samples to keep for averaging
const SAMPLE_WINDOW: usize = 10;

/// Threshold for considering a peer "fast" (bytes per second)
const FAST_PEER_THRESHOLD: f64 = 500_000.0; // 500KB/s

/// Metrics for a single peer
#[derive(Debug, Clone)]
pub struct PeerPerformance {
    /// Recent response times (most recent last)
    pub response_times: Vec<Duration>,
    /// Recent response sizes in bytes
    pub response_sizes: Vec<u64>,
    /// Total successful requests
    pub success_count: u64,
    /// Total failed/timed out requests
    pub failure_count: u64,
    /// Last request timestamp
    pub last_request: Option<Instant>,
    /// Current adaptive request size for this peer
    pub adaptive_request_bytes: u64,
}

impl Default for PeerPerformance {
    fn default() -> Self {
        Self {
            response_times: Vec::with_capacity(SAMPLE_WINDOW),
            response_sizes: Vec::with_capacity(SAMPLE_WINDOW),
            success_count: 0,
            failure_count: 0,
            last_request: None,
            adaptive_request_bytes: DEFAULT_REQUEST_BYTES,
        }
    }
}

impl PeerPerformance {
    /// Record a successful request
    pub fn record_success(&mut self, response_time: Duration, response_bytes: u64) {
        // Add new samples
        self.response_times.push(response_time);
        self.response_sizes.push(response_bytes);

        // Keep only recent samples
        if self.response_times.len() > SAMPLE_WINDOW {
            self.response_times.remove(0);
            self.response_sizes.remove(0);
        }

        self.success_count += 1;
        self.last_request = Some(Instant::now());

        // Update adaptive request size
        self.update_adaptive_size();
    }

    /// Record a failed request
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_request = Some(Instant::now());

        // Reduce request size on failure
        self.adaptive_request_bytes = (self.adaptive_request_bytes * 3 / 4).max(MIN_REQUEST_BYTES);
    }

    /// Calculate average throughput in bytes per second
    pub fn avg_throughput(&self) -> Option<f64> {
        if self.response_times.is_empty() {
            return None;
        }

        let total_bytes: u64 = self.response_sizes.iter().sum();
        let total_time: f64 = self.response_times.iter().map(|d| d.as_secs_f64()).sum();

        if total_time > 0.0 {
            Some(total_bytes as f64 / total_time)
        } else {
            None
        }
    }

    /// Calculate average response time
    pub fn avg_response_time(&self) -> Option<Duration> {
        if self.response_times.is_empty() {
            return None;
        }

        let total: Duration = self.response_times.iter().sum();
        Some(total / self.response_times.len() as u32)
    }

    /// Check if this peer is considered "fast"
    pub fn is_fast(&self) -> bool {
        self.avg_throughput()
            .map(|t| t >= FAST_PEER_THRESHOLD)
            .unwrap_or(false)
    }

    /// Update adaptive request size based on performance
    fn update_adaptive_size(&mut self) {
        let Some(throughput) = self.avg_throughput() else {
            return;
        };

        // Scale request size based on throughput
        // Fast peers get larger requests, slow peers get smaller ones
        let target_size = if throughput >= FAST_PEER_THRESHOLD * 2.0 {
            MAX_REQUEST_BYTES
        } else if throughput >= FAST_PEER_THRESHOLD {
            DEFAULT_REQUEST_BYTES * 3 / 2 // 768KB
        } else if throughput >= FAST_PEER_THRESHOLD / 2.0 {
            DEFAULT_REQUEST_BYTES
        } else {
            MIN_REQUEST_BYTES
        };

        // Gradually move towards target (smoothing)
        if target_size > self.adaptive_request_bytes {
            // Increase slowly
            self.adaptive_request_bytes =
                ((self.adaptive_request_bytes as f64 * 0.8 + target_size as f64 * 0.2) as u64)
                    .min(MAX_REQUEST_BYTES);
        } else {
            // Decrease faster
            self.adaptive_request_bytes =
                ((self.adaptive_request_bytes as f64 * 0.5 + target_size as f64 * 0.5) as u64)
                    .max(MIN_REQUEST_BYTES);
        }
    }

    /// Get the success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 1.0; // Assume good for new peers
        }
        self.success_count as f64 / total as f64
    }
}

/// Global peer metrics tracker
#[derive(Debug, Clone, Default)]
pub struct PeerMetrics {
    metrics: Arc<RwLock<HashMap<H256, PeerPerformance>>>,
}

impl PeerMetrics {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Record a successful request for a peer
    pub fn record_success(&self, peer_id: H256, response_time: Duration, response_bytes: u64) {
        let mut metrics = self.metrics.write();
        metrics
            .entry(peer_id)
            .or_default()
            .record_success(response_time, response_bytes);
    }

    /// Record a failed request for a peer
    pub fn record_failure(&self, peer_id: H256) {
        let mut metrics = self.metrics.write();
        metrics.entry(peer_id).or_default().record_failure();
    }

    /// Get the adaptive request size for a peer
    pub fn get_request_bytes(&self, peer_id: H256) -> u64 {
        let metrics = self.metrics.read();
        metrics
            .get(&peer_id)
            .map(|p| p.adaptive_request_bytes)
            .unwrap_or(DEFAULT_REQUEST_BYTES)
    }

    /// Get performance metrics for a peer
    pub fn get_performance(&self, peer_id: H256) -> Option<PeerPerformance> {
        let metrics = self.metrics.read();
        metrics.get(&peer_id).cloned()
    }

    /// Get all fast peers (sorted by throughput, fastest first)
    pub fn get_fast_peers(&self) -> Vec<(H256, PeerPerformance)> {
        let metrics = self.metrics.read();
        let mut fast_peers: Vec<_> = metrics
            .iter()
            .filter(|(_, perf)| perf.is_fast())
            .map(|(id, perf)| (*id, perf.clone()))
            .collect();

        // Sort by throughput (highest first)
        fast_peers.sort_by(|a, b| {
            b.1.avg_throughput()
                .partial_cmp(&a.1.avg_throughput())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        fast_peers
    }

    /// Get peers sorted by success rate and throughput (best first)
    pub fn get_ranked_peers(&self) -> Vec<(H256, f64)> {
        let metrics = self.metrics.read();
        let mut ranked: Vec<_> = metrics
            .iter()
            .map(|(id, perf)| {
                // Score = success_rate * normalized_throughput
                let throughput_score = perf
                    .avg_throughput()
                    .map(|t| (t / FAST_PEER_THRESHOLD).min(2.0))
                    .unwrap_or(1.0);
                let score = perf.success_rate() * throughput_score;
                (*id, score)
            })
            .collect();

        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }

    /// Remove metrics for disconnected peers
    pub fn remove_peer(&self, peer_id: H256) {
        let mut metrics = self.metrics.write();
        metrics.remove(&peer_id);
    }

    /// Get summary statistics
    pub fn summary(&self) -> PeerMetricsSummary {
        let metrics = self.metrics.read();
        let total_peers = metrics.len();
        let fast_peers = metrics.values().filter(|p| p.is_fast()).count();
        let avg_throughput = if total_peers > 0 {
            metrics
                .values()
                .filter_map(|p| p.avg_throughput())
                .sum::<f64>()
                / total_peers as f64
        } else {
            0.0
        };

        PeerMetricsSummary {
            total_peers,
            fast_peers,
            avg_throughput,
        }
    }
}

/// Summary of peer metrics
#[derive(Debug, Clone)]
pub struct PeerMetricsSummary {
    pub total_peers: usize,
    pub fast_peers: usize,
    pub avg_throughput: f64,
}

/// Global instance of peer metrics
pub static PEER_METRICS: once_cell::sync::Lazy<PeerMetrics> =
    once_cell::sync::Lazy::new(PeerMetrics::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_performance_recording() {
        let mut perf = PeerPerformance::default();

        // Record some successes
        perf.record_success(Duration::from_millis(100), 512 * 1024);
        perf.record_success(Duration::from_millis(150), 512 * 1024);

        assert_eq!(perf.success_count, 2);
        assert!(perf.avg_response_time().is_some());
        assert!(perf.avg_throughput().is_some());
    }

    #[test]
    fn test_adaptive_sizing() {
        let mut perf = PeerPerformance::default();

        // Simulate a fast peer (high throughput)
        for _ in 0..10 {
            perf.record_success(Duration::from_millis(50), 1024 * 1024); // 1MB in 50ms = 20MB/s
        }

        // Should have increased request size
        assert!(perf.adaptive_request_bytes > DEFAULT_REQUEST_BYTES);
    }

    #[test]
    fn test_failure_reduces_size() {
        let mut perf = PeerPerformance::default();
        let initial_size = perf.adaptive_request_bytes;

        perf.record_failure();

        assert!(perf.adaptive_request_bytes < initial_size);
    }
}
