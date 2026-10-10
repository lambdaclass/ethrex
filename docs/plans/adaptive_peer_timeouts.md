# Adaptive Peer Timeouts - Implementation Plan

**Author:** Pablo Deymonnaz
**Date:** February 2026
**Status:** Draft
**Related Roadmap Item:** 1.7 Peer Connection Optimization

---

## Overview

This plan describes how to implement adaptive timeouts for peer requests in snap sync. Instead of using a fixed 15-second timeout for all peers, we'll track each peer's response latency and set timeouts dynamically based on their historical performance.

**Expected Impact:** 20-30% improvement in peer utilization, faster detection of slow/dead peers.

---

## Current State

### Fixed Timeout
```rust
// snap/constants.rs:63
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(15);
```

### Peer Data Structure
```rust
// discv4/peer_table.rs:117-131
pub struct PeerData {
    pub node: Node,
    pub record: Option<NodeRecord>,
    pub supported_capabilities: Vec<Capability>,
    pub is_connection_inbound: bool,
    pub connection: Option<PeerConnection>,
    score: i64,      // Range: -150 to 50
    requests: i64,   // Current in-flight count
}
```

### Request Flow
```rust
// peer_handler.rs:106-119
pub(crate) async fn make_request(
    peer_table: &mut PeerTable,
    peer_id: H256,
    connection: &mut PeerConnection,
    message: RLPxMessage,
    timeout: Duration,  // Currently always PEER_REPLY_TIMEOUT (15s)
) -> Result<RLPxMessage, PeerConnectionError> {
    peer_table.inc_requests(peer_id).await?;
    let result = connection.outgoing_request(message, timeout).await;
    peer_table.dec_requests(peer_id).await?;
    result
}
```

---

## Proposed Design

### 1. Add Latency Tracking to PeerData

```rust
// discv4/peer_table.rs

/// Rolling average for peer latency tracking
#[derive(Debug, Clone, Default)]
pub struct LatencyTracker {
    /// Exponential moving average of response times in milliseconds
    avg_ms: f64,
    /// Number of samples recorded (for initial ramp-up)
    samples: u32,
}

impl LatencyTracker {
    const ALPHA: f64 = 0.2;  // Weight for new samples (higher = more reactive)
    const MIN_SAMPLES: u32 = 3;  // Minimum samples before using adaptive timeout

    /// Record a new latency measurement
    pub fn record(&mut self, latency: Duration) {
        let ms = latency.as_millis() as f64;
        if self.samples == 0 {
            self.avg_ms = ms;
        } else {
            // Exponential moving average: new_avg = α * new_value + (1-α) * old_avg
            self.avg_ms = Self::ALPHA * ms + (1.0 - Self::ALPHA) * self.avg_ms;
        }
        self.samples = self.samples.saturating_add(1);
    }

    /// Get the adaptive timeout for this peer
    /// Returns None if not enough samples yet (use default timeout)
    pub fn adaptive_timeout(&self, multiplier: f64, max_timeout: Duration) -> Option<Duration> {
        if self.samples < Self::MIN_SAMPLES {
            return None;  // Not enough data, use default
        }

        let timeout_ms = (self.avg_ms * multiplier) as u64;
        let timeout = Duration::from_millis(timeout_ms);

        // Clamp to reasonable bounds
        Some(timeout.min(max_timeout))
    }

    /// Get the average latency (for metrics/debugging)
    pub fn average_ms(&self) -> f64 {
        self.avg_ms
    }

    /// Get sample count
    pub fn samples(&self) -> u32 {
        self.samples
    }
}

// Update PeerData
pub struct PeerData {
    pub node: Node,
    pub record: Option<NodeRecord>,
    pub supported_capabilities: Vec<Capability>,
    pub is_connection_inbound: bool,
    pub connection: Option<PeerConnection>,
    score: i64,
    requests: i64,
    latency: LatencyTracker,  // NEW
}
```

### 2. Add Constants for Adaptive Timeouts

```rust
// snap/constants.rs

/// Default timeout for peer responses (used when no latency data available)
pub const PEER_REPLY_TIMEOUT_DEFAULT: Duration = Duration::from_secs(15);

/// Minimum timeout regardless of measured latency
pub const PEER_REPLY_TIMEOUT_MIN: Duration = Duration::from_secs(2);

/// Maximum timeout regardless of measured latency
pub const PEER_REPLY_TIMEOUT_MAX: Duration = Duration::from_secs(30);

/// Multiplier applied to average latency to get timeout
/// e.g., 3.0 means timeout = 3x average response time
pub const PEER_TIMEOUT_MULTIPLIER: f64 = 3.0;
```

### 3. Add PeerTable Methods for Latency

```rust
// discv4/peer_table.rs

impl PeerTable {
    /// Record a successful response latency for a peer
    pub async fn record_latency(
        &mut self,
        node_id: H256,
        latency: Duration,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RecordLatency { node_id, latency })
            .await?;
        Ok(())
    }

    /// Get the adaptive timeout for a peer
    /// Returns default timeout if peer not found or not enough samples
    pub async fn get_peer_timeout(
        &self,
        node_id: H256,
    ) -> Result<Duration, PeerTableError> {
        self.handle
            .call(CallMessage::GetPeerTimeout { node_id })
            .await?
    }
}

// Add to CastMessage enum
enum CastMessage {
    // ... existing variants ...
    RecordLatency { node_id: H256, latency: Duration },
}

// Add to CallMessage enum
enum CallMessage {
    // ... existing variants ...
    GetPeerTimeout { node_id: H256 },
}

// Handle in PeerTableServer
impl PeerTableServer {
    fn handle_record_latency(&mut self, node_id: H256, latency: Duration) {
        if let Some(peer) = self.connected_peers.get_mut(&node_id) {
            peer.latency.record(latency);
        }
    }

    fn handle_get_peer_timeout(&self, node_id: H256) -> Duration {
        self.connected_peers
            .get(&node_id)
            .and_then(|peer| {
                peer.latency.adaptive_timeout(
                    PEER_TIMEOUT_MULTIPLIER,
                    PEER_REPLY_TIMEOUT_MAX,
                )
            })
            .unwrap_or(PEER_REPLY_TIMEOUT_DEFAULT)
            .max(PEER_REPLY_TIMEOUT_MIN)
    }
}
```

### 4. Update make_request to Track Latency

```rust
// peer_handler.rs

pub(crate) async fn make_request(
    peer_table: &mut PeerTable,
    peer_id: H256,
    connection: &mut PeerConnection,
    message: RLPxMessage,
    timeout: Duration,
) -> Result<RLPxMessage, PeerConnectionError> {
    peer_table.inc_requests(peer_id).await?;

    let start = std::time::Instant::now();
    let result = connection.outgoing_request(message, timeout).await;
    let elapsed = start.elapsed();

    peer_table.dec_requests(peer_id).await?;

    // Record latency on successful responses
    if result.is_ok() {
        // Fire and forget - don't fail the request if latency recording fails
        let _ = peer_table.record_latency(peer_id, elapsed).await;
    }

    result
}
```

### 5. Create Helper for Getting Adaptive Timeout

```rust
// peer_handler.rs or snap/client.rs

/// Get the timeout to use for a peer request
/// Uses adaptive timeout if available, otherwise falls back to default
pub async fn get_request_timeout(
    peer_table: &PeerTable,
    peer_id: H256,
) -> Duration {
    peer_table
        .get_peer_timeout(peer_id)
        .await
        .unwrap_or(PEER_REPLY_TIMEOUT_DEFAULT)
}
```

### 6. Update Call Sites

Replace hardcoded `PEER_REPLY_TIMEOUT` with adaptive timeout lookups.

**Example - snap/client.rs:request_state_trienodes()**
```rust
// Before
let response = PeerHandler::make_request(
    &mut peer_table,
    peer_id,
    &mut connection,
    RLPxMessage::GetTrieNodes(request),
    PEER_REPLY_TIMEOUT,  // Fixed 15s
).await;

// After
let timeout = peer_table.get_peer_timeout(peer_id).await
    .unwrap_or(PEER_REPLY_TIMEOUT_DEFAULT);
let response = PeerHandler::make_request(
    &mut peer_table,
    peer_id,
    &mut connection,
    RLPxMessage::GetTrieNodes(request),
    timeout,  // Adaptive
).await;
```

**Files to update:**
- `peer_handler.rs` - block header/body requests (~5 locations)
- `snap/client.rs` - account/storage/bytecode/trienode requests (~8 locations)

---

## Implementation Steps

### Step 1: Add LatencyTracker struct
- Create `LatencyTracker` in `discv4/peer_table.rs`
- Add unit tests for the EMA calculation
- **Effort:** 1 hour

### Step 2: Update PeerData and PeerTable
- Add `latency: LatencyTracker` field to `PeerData`
- Add `CastMessage::RecordLatency` and `CallMessage::GetPeerTimeout`
- Implement handlers in `PeerTableServer`
- Add `record_latency()` and `get_peer_timeout()` methods to `PeerTable`
- **Effort:** 2-3 hours

### Step 3: Add new constants
- Add timeout constants to `snap/constants.rs`
- Keep `PEER_REPLY_TIMEOUT` as `PEER_REPLY_TIMEOUT_DEFAULT` for backwards compatibility
- **Effort:** 30 minutes

### Step 4: Update make_request
- Add timing measurement
- Record latency on success
- **Effort:** 30 minutes

### Step 5: Update call sites
- Replace `PEER_REPLY_TIMEOUT` with adaptive timeout lookups
- Start with snap sync paths (highest impact)
- **Effort:** 2-3 hours

### Step 6: Add metrics (optional but recommended)
- Export peer latency averages to Prometheus
- Add histogram of timeouts used
- **Effort:** 1-2 hours

### Step 7: Testing
- Unit tests for LatencyTracker
- Integration tests with mock peers
- Manual testing on testnet
- **Effort:** 2-3 hours

**Total Effort:** ~2 days

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_tracker_initial() {
        let tracker = LatencyTracker::default();
        assert_eq!(tracker.samples(), 0);
        assert!(tracker.adaptive_timeout(3.0, Duration::from_secs(30)).is_none());
    }

    #[test]
    fn test_latency_tracker_needs_min_samples() {
        let mut tracker = LatencyTracker::default();
        tracker.record(Duration::from_millis(100));
        tracker.record(Duration::from_millis(100));
        // Only 2 samples, need 3
        assert!(tracker.adaptive_timeout(3.0, Duration::from_secs(30)).is_none());

        tracker.record(Duration::from_millis(100));
        // Now we have 3 samples
        let timeout = tracker.adaptive_timeout(3.0, Duration::from_secs(30));
        assert!(timeout.is_some());
    }

    #[test]
    fn test_latency_tracker_ema() {
        let mut tracker = LatencyTracker::default();
        // Record 5 samples of 100ms
        for _ in 0..5 {
            tracker.record(Duration::from_millis(100));
        }
        // Average should be ~100ms
        assert!((tracker.average_ms() - 100.0).abs() < 1.0);

        // Record a slow sample
        tracker.record(Duration::from_millis(500));
        // EMA should move towards 500 but not jump to it
        // With α=0.2: new_avg = 0.2 * 500 + 0.8 * 100 = 180
        assert!((tracker.average_ms() - 180.0).abs() < 1.0);
    }

    #[test]
    fn test_latency_tracker_timeout_clamped() {
        let mut tracker = LatencyTracker::default();
        for _ in 0..5 {
            tracker.record(Duration::from_millis(20000)); // 20 seconds
        }

        // 3x would be 60s, but max is 30s
        let timeout = tracker.adaptive_timeout(3.0, Duration::from_secs(30)).unwrap();
        assert_eq!(timeout, Duration::from_secs(30));
    }
}
```

### Integration Test Scenario

1. Start sync with multiple mock peers
2. One peer responds in 50ms, another in 500ms
3. Verify fast peer gets shorter timeouts (~150ms)
4. Verify slow peer gets longer timeouts (~1.5s)
5. Simulate peer getting slower - verify timeout adapts
6. Simulate peer timing out - verify fallback to default

---

## Rollout Strategy

### Phase 1: Feature Flag (Optional)
Add a feature flag to enable/disable adaptive timeouts:
```rust
#[cfg(feature = "adaptive-timeouts")]
let timeout = peer_table.get_peer_timeout(peer_id).await...;

#[cfg(not(feature = "adaptive-timeouts"))]
let timeout = PEER_REPLY_TIMEOUT_DEFAULT;
```

### Phase 2: Testnet Validation
- Deploy to Sepolia/Holesky
- Monitor sync times and peer utilization
- Check for any regressions

### Phase 3: Mainnet
- Enable for mainnet snap sync
- Monitor metrics
- Adjust constants if needed (multiplier, min/max bounds)

---

## Metrics to Add

```rust
// metrics.rs

/// Histogram of peer latencies
pub static PEER_LATENCY_HISTOGRAM: Lazy<Histogram> = Lazy::new(|| {
    Histogram::with_opts(
        HistogramOpts::new("snap_peer_latency_ms", "Peer response latency in milliseconds")
            .buckets(vec![10.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 15000.0])
    ).unwrap()
});

/// Gauge of current adaptive timeout per peer (for debugging)
pub static PEER_ADAPTIVE_TIMEOUT: Lazy<GaugeVec> = Lazy::new(|| {
    GaugeVec::new(
        Opts::new("snap_peer_adaptive_timeout_ms", "Current adaptive timeout for peer"),
        &["peer_id"]
    ).unwrap()
});
```

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Too aggressive timeouts cause premature disconnects | High | Use conservative multiplier (3x), enforce minimum timeout (2s) |
| Latency spikes cause permanent high timeouts | Medium | EMA naturally decays; consider adding explicit decay |
| Memory overhead from tracking all peers | Low | LatencyTracker is only 16 bytes per peer |
| Breaking change if constants renamed | Low | Keep old constant names as aliases |

---

## Future Enhancements

1. **Latency by request type**: Track separate latencies for account ranges vs trie nodes (different expected response times)

2. **Peer quality score integration**: Factor latency into the existing peer scoring system

3. **Timeout decay**: If a peer hasn't been used recently, gradually reset towards default timeout

4. **Request-size based timeout**: Larger requests (more accounts/nodes) should have longer timeouts

---

## Appendix: Call Sites to Update

| File | Function | Line (approx) |
|------|----------|---------------|
| `peer_handler.rs` | `ask_peer_head_number()` | 70 |
| `peer_handler.rs` | `request_block_headers_from_hash()` | 415 |
| `peer_handler.rs` | `download_chunk_from_peer()` | 462 |
| `peer_handler.rs` | `request_block_bodies_inner()` | 501 |
| `peer_handler.rs` | `get_block_header()` | 590 |
| `snap/client.rs` | `request_account_range_worker()` | 1188 |
| `snap/client.rs` | `request_storage_ranges_worker()` | 1302 |
| `snap/client.rs` | `request_bytecodes()` | 481 |
| `snap/client.rs` | `request_state_trienodes()` | 1084 |
| `snap/client.rs` | `request_storage_trienodes()` | 1140 |

---

*Document Version: 1.0*
*Last Updated: February 2026*
