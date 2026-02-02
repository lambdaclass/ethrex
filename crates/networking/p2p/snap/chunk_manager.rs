//! Adaptive Chunk Manager for Snap Sync
//!
//! This module provides adaptive chunk sizing for account range downloads.
//! It tracks peer performance and adjusts chunk sizes to optimize throughput.

use ethrex_common::{H256, U256};
use std::collections::HashMap;

/// Minimum chunk size to avoid overly granular requests.
/// Represents a fraction of the total address space.
const MIN_CHUNK_SIZE_DIVISOR: u64 = 100_000;

/// Maximum chunk size to ensure reasonable response times.
/// Represents a fraction of the total address space.
const MAX_CHUNK_SIZE_DIVISOR: u64 = 100;

/// Default chunk count when no peer performance data is available.
const DEFAULT_CHUNK_COUNT: usize = 800;

/// EMA alpha for throughput smoothing (0.3 = 30% weight to new measurements).
const THROUGHPUT_EMA_ALPHA: f64 = 0.3;

/// Minimum number of responses before adjusting chunk size.
const MIN_RESPONSES_FOR_ADJUSTMENT: u64 = 3;

/// Peer performance tracking data.
#[derive(Debug, Clone)]
struct PeerPerformance {
    /// Exponential moving average of accounts per millisecond.
    throughput_ema: f64,
    /// Number of successful responses from this peer.
    response_count: u64,
    /// Current chunk size for this peer (as a divisor of the address space).
    chunk_size_divisor: u64,
}

impl Default for PeerPerformance {
    fn default() -> Self {
        Self {
            throughput_ema: 0.0,
            response_count: 0,
            // Start with a moderate chunk size
            chunk_size_divisor: DEFAULT_CHUNK_COUNT as u64,
        }
    }
}

/// Manages adaptive chunk sizing for account range downloads.
///
/// The chunk manager tracks per-peer performance and adjusts chunk sizes
/// to optimize download throughput. Faster peers get larger chunks while
/// slower peers get smaller chunks.
#[derive(Debug)]
pub struct AdaptiveChunkManager {
    /// Performance data for each peer.
    peer_performance: HashMap<H256, PeerPerformance>,
    /// Total address space range being downloaded.
    total_range: U256,
    /// Minimum allowed chunk size.
    min_chunk_size: U256,
    /// Maximum allowed chunk size.
    max_chunk_size: U256,
}

impl AdaptiveChunkManager {
    /// Creates a new AdaptiveChunkManager for the given address range.
    ///
    /// # Arguments
    /// * `start` - Starting hash of the range
    /// * `limit` - Ending hash of the range
    pub fn new(start: H256, limit: H256) -> Self {
        let start_u256 = U256::from_big_endian(&start.0);
        let limit_u256 = U256::from_big_endian(&limit.0);
        let total_range = limit_u256.saturating_sub(start_u256);

        // Calculate min/max chunk sizes based on the total range
        let min_chunk_size = total_range / MIN_CHUNK_SIZE_DIVISOR;
        let max_chunk_size = total_range / MAX_CHUNK_SIZE_DIVISOR;

        Self {
            peer_performance: HashMap::new(),
            total_range,
            min_chunk_size: min_chunk_size.max(U256::one()),
            max_chunk_size: max_chunk_size.max(min_chunk_size),
        }
    }

    /// Gets the recommended chunk size for a specific peer.
    ///
    /// Returns a chunk size optimized for the peer's historical performance.
    /// New peers get a default chunk size until we have performance data.
    pub fn get_chunk_size_for_peer(&self, peer_id: &H256) -> U256 {
        let divisor = self
            .peer_performance
            .get(peer_id)
            .map(|p| p.chunk_size_divisor)
            .unwrap_or(DEFAULT_CHUNK_COUNT as u64);

        let chunk_size = self.total_range / divisor.max(1);

        // Clamp to min/max bounds
        chunk_size.max(self.min_chunk_size).min(self.max_chunk_size)
    }

    /// Records a response from a peer and updates their performance metrics.
    ///
    /// # Arguments
    /// * `peer_id` - The peer that responded
    /// * `accounts_received` - Number of accounts in the response
    /// * `duration_ms` - Time taken to receive the response in milliseconds
    pub fn record_response(&mut self, peer_id: H256, accounts_received: usize, duration_ms: u64) {
        // Avoid division by zero
        let duration_ms = duration_ms.max(1);

        // Calculate throughput (accounts per millisecond)
        let throughput = accounts_received as f64 / duration_ms as f64;

        let perf = self.peer_performance.entry(peer_id).or_default();

        // Update throughput EMA
        if perf.response_count == 0 {
            perf.throughput_ema = throughput;
        } else {
            perf.throughput_ema = THROUGHPUT_EMA_ALPHA * throughput
                + (1.0 - THROUGHPUT_EMA_ALPHA) * perf.throughput_ema;
        }

        perf.response_count += 1;

        // Adjust chunk size based on performance after gathering enough data
        if perf.response_count >= MIN_RESPONSES_FOR_ADJUSTMENT {
            self.adjust_chunk_size(&peer_id);
        }
    }

    /// Adjusts the chunk size for a peer based on their throughput.
    fn adjust_chunk_size(&mut self, peer_id: &H256) {
        let Some(perf) = self.peer_performance.get_mut(peer_id) else {
            return;
        };

        // Target: complete a chunk in about 5 seconds (5000ms)
        // chunk_accounts = throughput * target_time
        // chunk_size = total_range * (chunk_accounts / estimated_total_accounts)
        //
        // Simplified: adjust divisor based on throughput relative to baseline
        // Higher throughput = lower divisor = larger chunks

        // Baseline throughput: 1 account per ms (1000 accounts/sec)
        const BASELINE_THROUGHPUT: f64 = 1.0;

        let throughput_ratio = perf.throughput_ema / BASELINE_THROUGHPUT;

        // Adjust divisor: faster peers get smaller divisors (larger chunks)
        // Clamp the ratio to prevent extreme adjustments
        let clamped_ratio = throughput_ratio.clamp(0.1, 10.0);

        // New divisor = base_divisor / throughput_ratio
        // (faster peer -> smaller divisor -> larger chunk)
        let new_divisor = (DEFAULT_CHUNK_COUNT as f64 / clamped_ratio) as u64;

        // Clamp to valid range
        perf.chunk_size_divisor = new_divisor.clamp(MAX_CHUNK_SIZE_DIVISOR, MIN_CHUNK_SIZE_DIVISOR);
    }

    /// Returns the default chunk count for initial task creation.
    pub fn default_chunk_count() -> usize {
        DEFAULT_CHUNK_COUNT
    }

    /// Gets performance statistics for a peer (for debugging/logging).
    pub fn get_peer_stats(&self, peer_id: &H256) -> Option<(f64, u64)> {
        self.peer_performance
            .get(peer_id)
            .map(|p| (p.throughput_ema, p.response_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_chunk_manager() {
        let start = H256::zero();
        let limit = H256::repeat_byte(0xff);

        let manager = AdaptiveChunkManager::new(start, limit);

        assert!(manager.min_chunk_size > U256::zero());
        assert!(manager.max_chunk_size >= manager.min_chunk_size);
    }

    #[test]
    fn test_default_chunk_size() {
        let start = H256::zero();
        let limit = H256::repeat_byte(0xff);
        let manager = AdaptiveChunkManager::new(start, limit);

        let peer_id = H256::random();
        let chunk_size = manager.get_chunk_size_for_peer(&peer_id);

        // Should return a valid chunk size
        assert!(chunk_size >= manager.min_chunk_size);
        assert!(chunk_size <= manager.max_chunk_size);
    }

    #[test]
    fn test_record_response_updates_performance() {
        let start = H256::zero();
        let limit = H256::repeat_byte(0xff);
        let mut manager = AdaptiveChunkManager::new(start, limit);

        let peer_id = H256::random();

        // Record a fast response
        manager.record_response(peer_id, 1000, 100); // 10 accounts/ms

        let stats = manager.get_peer_stats(&peer_id);
        assert!(stats.is_some());

        let (throughput, count) = stats.unwrap();
        assert!(throughput > 0.0);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_chunk_size_adjusts_for_fast_peer() {
        let start = H256::zero();
        let limit = H256::repeat_byte(0xff);
        let mut manager = AdaptiveChunkManager::new(start, limit);

        let fast_peer = H256::random();
        let slow_peer = H256::random();

        // Fast peer: 5000 accounts in 500ms = 10 accounts/ms
        for _ in 0..5 {
            manager.record_response(fast_peer, 5000, 500);
        }

        // Slow peer: 500 accounts in 5000ms = 0.1 accounts/ms
        for _ in 0..5 {
            manager.record_response(slow_peer, 500, 5000);
        }

        let fast_chunk = manager.get_chunk_size_for_peer(&fast_peer);
        let slow_chunk = manager.get_chunk_size_for_peer(&slow_peer);

        // Fast peer should get larger chunks
        assert!(
            fast_chunk > slow_chunk,
            "Fast peer chunk {} should be larger than slow peer chunk {}",
            fast_chunk,
            slow_chunk
        );
    }

    #[test]
    fn test_chunk_size_bounded() {
        let start = H256::zero();
        let limit = H256::repeat_byte(0xff);
        let mut manager = AdaptiveChunkManager::new(start, limit);

        let peer_id = H256::random();

        // Extremely fast peer
        for _ in 0..10 {
            manager.record_response(peer_id, 100000, 10);
        }

        let chunk_size = manager.get_chunk_size_for_peer(&peer_id);
        assert!(chunk_size <= manager.max_chunk_size);
        assert!(chunk_size >= manager.min_chunk_size);
    }
}
