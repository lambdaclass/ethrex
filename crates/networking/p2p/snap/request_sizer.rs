//! Adaptive request sizing for snap sync protocol.
//!
//! Adjusts the `response_bytes` budget per peer based on observed response latency,
//! similar to Nethermind's adaptive approach. Fast peers get larger requests,
//! slow peers get smaller ones.

use super::constants::{
    INITIAL_RESPONSE_BYTES, MAX_RESPONSE_BYTES_ADAPTIVE, MIN_RESPONSE_BYTES_ADAPTIVE,
};
use ethrex_common::H256;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Latency threshold below which the budget is scaled up.
const LATENCY_LOW: Duration = Duration::from_secs(2);

/// Latency threshold above which the budget is scaled down.
const LATENCY_HIGH: Duration = Duration::from_secs(10);

/// Scale factor: multiply by 3/2 (1.5x) for scale-up, multiply by 2/3 for scale-down.
const SCALE_UP_NUM: u64 = 3;
const SCALE_UP_DEN: u64 = 2;

/// Per-peer adaptive request sizer state.
#[derive(Debug, Clone)]
struct PeerSizer {
    response_bytes: u64,
}

impl PeerSizer {
    fn new() -> Self {
        Self {
            response_bytes: INITIAL_RESPONSE_BYTES,
        }
    }

    fn response_bytes(&self) -> u64 {
        self.response_bytes
    }

    fn record_response(&mut self, latency: Duration) {
        if latency < LATENCY_LOW {
            // Scale up: multiply by 1.5, cap at max
            self.response_bytes = (self.response_bytes * SCALE_UP_NUM / SCALE_UP_DEN)
                .min(MAX_RESPONSE_BYTES_ADAPTIVE);
        } else if latency > LATENCY_HIGH {
            // Scale down: divide by 1.5, floor at min
            self.response_bytes = (self.response_bytes * SCALE_UP_DEN / SCALE_UP_NUM)
                .max(MIN_RESPONSE_BYTES_ADAPTIVE);
        }
    }
}

/// Thread-safe map of per-peer adaptive request sizers.
///
/// Cheaply cloneable (Arc-backed). Pass clones to spawned tasks.
#[derive(Debug, Clone, Default)]
pub struct RequestSizerMap {
    inner: Arc<Mutex<HashMap<H256, PeerSizer>>>,
}

impl RequestSizerMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current response bytes budget for a peer.
    /// Creates a new sizer at the default budget if the peer is not yet tracked.
    pub fn response_bytes_for_peer(&self, peer_id: &H256) -> u64 {
        self.inner
            .lock()
            .expect("RequestSizerMap lock poisoned")
            .entry(*peer_id)
            .or_insert_with(PeerSizer::new)
            .response_bytes()
    }

    /// Records a response latency for a peer, adjusting its future request budget.
    pub fn record_response(&self, peer_id: &H256, latency: Duration) {
        self.inner
            .lock()
            .expect("RequestSizerMap lock poisoned")
            .entry(*peer_id)
            .or_insert_with(PeerSizer::new)
            .record_response(latency);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_initial_budget() {
        let sizer = RequestSizerMap::new();
        let peer = H256::zero();
        assert_eq!(sizer.response_bytes_for_peer(&peer), INITIAL_RESPONSE_BYTES);
    }

    #[test]
    fn scales_up_on_fast_response() {
        let sizer = RequestSizerMap::new();
        let peer = H256::zero();
        sizer.record_response(&peer, Duration::from_secs(1));
        let expected = INITIAL_RESPONSE_BYTES * 3 / 2;
        assert_eq!(sizer.response_bytes_for_peer(&peer), expected);
    }

    #[test]
    fn scales_down_on_slow_response() {
        let sizer = RequestSizerMap::new();
        let peer = H256::zero();
        sizer.record_response(&peer, Duration::from_secs(11));
        let expected = INITIAL_RESPONSE_BYTES * 2 / 3;
        assert_eq!(sizer.response_bytes_for_peer(&peer), expected);
    }

    #[test]
    fn no_change_in_normal_range() {
        let sizer = RequestSizerMap::new();
        let peer = H256::zero();
        sizer.record_response(&peer, Duration::from_secs(5));
        assert_eq!(sizer.response_bytes_for_peer(&peer), INITIAL_RESPONSE_BYTES);
    }

    #[test]
    fn clamps_to_max() {
        let sizer = RequestSizerMap::new();
        let peer = H256::zero();
        // Scale up many times
        for _ in 0..50 {
            sizer.record_response(&peer, Duration::from_millis(100));
        }
        assert_eq!(
            sizer.response_bytes_for_peer(&peer),
            MAX_RESPONSE_BYTES_ADAPTIVE
        );
    }

    #[test]
    fn clamps_to_min() {
        let sizer = RequestSizerMap::new();
        let peer = H256::zero();
        // Scale down many times
        for _ in 0..50 {
            sizer.record_response(&peer, Duration::from_secs(30));
        }
        assert_eq!(
            sizer.response_bytes_for_peer(&peer),
            MIN_RESPONSE_BYTES_ADAPTIVE
        );
    }

    #[test]
    fn independent_per_peer() {
        let sizer = RequestSizerMap::new();
        let fast_peer = H256::from_low_u64_be(1);
        let slow_peer = H256::from_low_u64_be(2);

        sizer.record_response(&fast_peer, Duration::from_millis(500));
        sizer.record_response(&slow_peer, Duration::from_secs(15));

        assert!(sizer.response_bytes_for_peer(&fast_peer) > INITIAL_RESPONSE_BYTES);
        assert!(sizer.response_bytes_for_peer(&slow_peer) < INITIAL_RESPONSE_BYTES);
    }
}
