//! Enhanced multi-dimensional peer scoring system.
//!
//! This module implements a sophisticated peer scoring system inspired by
//! Lighthouse PeerManager and libp2p's Gossipsub scoring (P1-P7 factors).
//!
//! ## Key Features
//!
//! - **Multi-dimensional scoring**: Tracks separate metrics per request type
//! - **Latency tracking**: EWMA and percentile-based latency tracking (p50, p95, p99)
//! - **Throughput tracking**: Bytes per second for data-heavy requests
//! - **Time-weighted decay**: Exponential decay with configurable half-life
//! - **Confidence intervals**: New peers blend toward neutral until enough interactions
//! - **Sybil protection**: IP colocation penalty for same /24 prefix peers
//!
//! ## Usage
//!
//! ```rust,ignore
//! use ethrex_p2p::scoring::{PeerScore, RequestType, FailureSeverity, PeerScoringConfig};
//!
//! let mut score = PeerScore::new(PeerScoringConfig::default());
//!
//! // Record successful request
//! score.record_success(RequestType::BlockHeaders, Duration::from_millis(100), Some(1024));
//!
//! // Record failed request
//! score.record_failure(RequestType::BlockBodies, FailureSeverity::Medium);
//!
//! // Get composite score for request type selection
//! let composite = score.compute_composite_score(RequestType::BlockHeaders);
//! ```

mod ip_tracker;
mod metrics;
mod score;
mod types;

pub use ip_tracker::{IpColocationTracker, IpPrefix};
pub use metrics::{LatencyTracker, RequestTypeMetrics, ThroughputTracker, EWMA};
pub use score::{PeerScore, PeerScoringConfig};
pub use types::{FailureSeverity, RequestType};
