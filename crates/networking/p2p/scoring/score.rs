//! Peer scoring implementation with multi-dimensional tracking.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::metrics::RequestTypeMetrics;
use super::types::{FailureSeverity, RequestType};

/// Default decay half-life in seconds (1 hour).
const DEFAULT_DECAY_HALF_LIFE_SECS: u64 = 3600;

/// Minimum interactions required for full confidence.
const DEFAULT_MIN_INTERACTIONS: u64 = 10;

/// Reference latency for normalization (100ms = perfect, 1000ms = zero score).
const LATENCY_REFERENCE_MS: f64 = 100.0;
const LATENCY_MAX_MS: f64 = 1000.0;

/// Reference throughput for normalization (1MB/s = perfect score).
const THROUGHPUT_REFERENCE_BPS: f64 = 1024.0 * 1024.0;

/// Configuration for peer scoring behavior.
#[derive(Debug, Clone)]
pub struct PeerScoringConfig {
    /// Half-life for score decay in seconds.
    /// After this duration, scores decay by 50%.
    pub decay_half_life_secs: u64,

    /// Minimum interactions required before confidence reaches 1.0.
    pub min_interactions_for_confidence: u64,

    /// Weight for reliability in composite score (0.0 - 1.0).
    pub reliability_weight: f64,

    /// Weight for latency in composite score (0.0 - 1.0).
    pub latency_weight: f64,

    /// Weight for throughput in composite score (0.0 - 1.0).
    pub throughput_weight: f64,

    /// Weight for compliance in composite score (0.0 - 1.0).
    pub compliance_weight: f64,

    /// Maximum number of concurrent requests allowed per peer (base value).
    /// Actual limit is scaled by score.
    pub max_concurrent_base: i64,
}

impl Default for PeerScoringConfig {
    fn default() -> Self {
        Self {
            decay_half_life_secs: DEFAULT_DECAY_HALF_LIFE_SECS,
            min_interactions_for_confidence: DEFAULT_MIN_INTERACTIONS,
            reliability_weight: 0.40,
            latency_weight: 0.25,
            throughput_weight: 0.20,
            compliance_weight: 0.15,
            max_concurrent_base: 100,
        }
    }
}

impl PeerScoringConfig {
    /// Validates that weights sum to approximately 1.0.
    pub fn validate(&self) -> bool {
        let sum = self.reliability_weight
            + self.latency_weight
            + self.throughput_weight
            + self.compliance_weight;
        (sum - 1.0).abs() < 0.01
    }
}

/// Multi-dimensional peer score.
///
/// Tracks performance metrics per request type and computes composite
/// scores with time decay and confidence intervals.
#[derive(Debug, Clone)]
pub struct PeerScore {
    /// Metrics per request type
    request_metrics: HashMap<RequestType, RequestTypeMetrics>,

    /// Protocol compliance score (0.0 - 1.0).
    /// Tracks whether peer follows protocol correctly.
    compliance_score: f64,

    /// Decayed reliability component (accumulates over time with decay).
    decayed_reliability: f64,

    /// Last time decay was applied.
    last_decay: Instant,

    /// When this peer was first connected.
    connected_at: Instant,

    /// Whether this peer is blacklisted due to critical failure.
    is_blacklisted: bool,

    /// Configuration for scoring behavior.
    config: PeerScoringConfig,
}

impl Default for PeerScore {
    fn default() -> Self {
        Self::new(PeerScoringConfig::default())
    }
}

impl PeerScore {
    /// Creates a new peer score with the given configuration.
    pub fn new(config: PeerScoringConfig) -> Self {
        let now = Instant::now();
        Self {
            request_metrics: HashMap::new(),
            compliance_score: 1.0,    // Start with perfect compliance
            decayed_reliability: 0.5, // Start neutral
            last_decay: now,
            connected_at: now,
            is_blacklisted: false,
            config,
        }
    }

    /// Records a successful request.
    pub fn record_success(
        &mut self,
        request_type: RequestType,
        latency: Duration,
        bytes: Option<u64>,
    ) {
        self.apply_decay();

        let metrics = self.request_metrics.entry(request_type).or_default();

        metrics.record_success(latency, bytes);

        // Boost decayed reliability slightly
        self.decayed_reliability = (self.decayed_reliability + 0.01).min(1.0);
    }

    /// Records a failed request with severity.
    pub fn record_failure(&mut self, request_type: RequestType, severity: FailureSeverity) {
        self.apply_decay();

        let metrics = self.request_metrics.entry(request_type).or_default();

        metrics.record_failure();

        // Apply penalty based on severity
        let penalty = severity.penalty();
        self.decayed_reliability = (self.decayed_reliability + penalty).clamp(0.0, 1.0);

        // Critical failures impact compliance
        if severity >= FailureSeverity::High {
            self.compliance_score = (self.compliance_score - 0.1).max(0.0);
        }

        // Blacklist on critical
        if severity.is_blacklistable() {
            self.is_blacklisted = true;
        }
    }

    /// Applies time-based decay to scores.
    fn apply_decay(&mut self) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_decay);

        if elapsed.as_secs() < 10 {
            // Don't decay too frequently
            return;
        }

        let half_life = Duration::from_secs(self.config.decay_half_life_secs);
        let decay_factor = 0.5_f64.powf(elapsed.as_secs_f64() / half_life.as_secs_f64());

        // Decay reliability toward neutral (0.5)
        self.decayed_reliability = 0.5 + (self.decayed_reliability - 0.5) * decay_factor;

        // Slowly recover compliance
        self.compliance_score = self.compliance_score + (1.0 - self.compliance_score) * 0.01;

        self.last_decay = now;
    }

    /// Computes the composite score for a specific request type.
    ///
    /// Returns a value between 0.0 (worst) and 1.0 (best).
    pub fn compute_composite_score(&self, request_type: RequestType) -> f64 {
        if self.is_blacklisted {
            return 0.0;
        }

        let metrics = self
            .request_metrics
            .get(&request_type)
            .cloned()
            .unwrap_or_default();

        // Calculate individual components
        let reliability = self.compute_reliability(&metrics);
        let latency_score = self.compute_latency_score(&metrics);
        let throughput_score = self.compute_throughput_score(&metrics);

        // Weighted combination
        let weighted_score = self.config.reliability_weight * reliability
            + self.config.latency_weight * latency_score
            + self.config.throughput_weight * throughput_score
            + self.config.compliance_weight * self.compliance_score;

        // Apply confidence scaling
        let confidence = self.compute_confidence(&metrics);
        confidence * weighted_score + (1.0 - confidence) * 0.5
    }

    /// Computes an overall composite score (average across all request types).
    pub fn compute_overall_score(&self) -> f64 {
        if self.is_blacklisted {
            return 0.0;
        }

        if self.request_metrics.is_empty() {
            // No data yet, return neutral with confidence penalty
            let confidence = 0.0;
            return confidence * 0.5 + (1.0 - confidence) * 0.5;
        }

        let sum: f64 = RequestType::all()
            .iter()
            .map(|rt| self.compute_composite_score(*rt))
            .sum();

        sum / RequestType::all().len() as f64
    }

    /// Computes reliability score combining success rate and decayed component.
    fn compute_reliability(&self, metrics: &RequestTypeMetrics) -> f64 {
        let success_rate = metrics.success_rate();

        // Blend success rate with decayed reliability
        0.7 * success_rate + 0.3 * self.decayed_reliability
    }

    /// Computes latency score (1.0 for fast, 0.0 for slow).
    fn compute_latency_score(&self, metrics: &RequestTypeMetrics) -> f64 {
        let Some(ewma_ms) = metrics.latency.ewma_ms() else {
            return 0.5; // Neutral if no data
        };

        // Linear interpolation: 100ms = 1.0, 1000ms = 0.0
        let score =
            1.0 - (ewma_ms - LATENCY_REFERENCE_MS) / (LATENCY_MAX_MS - LATENCY_REFERENCE_MS);
        score.clamp(0.0, 1.0)
    }

    /// Computes throughput score (1.0 for fast, 0.0 for slow).
    fn compute_throughput_score(&self, metrics: &RequestTypeMetrics) -> f64 {
        let Some(bps) = metrics.throughput.bytes_per_sec() else {
            return 0.5; // Neutral if no data
        };

        // Normalized: 1MB/s = 1.0, asymptotic approach
        let score = (bps / THROUGHPUT_REFERENCE_BPS).min(1.0);
        score.clamp(0.0, 1.0)
    }

    /// Computes confidence based on number of interactions.
    fn compute_confidence(&self, metrics: &RequestTypeMetrics) -> f64 {
        let total = metrics.total_requests();
        let min_required = self.config.min_interactions_for_confidence;

        (total as f64 / min_required as f64).min(1.0)
    }

    /// Returns the total number of requests across all types.
    pub fn total_requests(&self) -> u64 {
        self.request_metrics
            .values()
            .map(|m| m.total_requests())
            .sum()
    }

    /// Returns the total successes across all types.
    pub fn total_successes(&self) -> u64 {
        self.request_metrics.values().map(|m| m.successes).sum()
    }

    /// Returns the total failures across all types.
    pub fn total_failures(&self) -> u64 {
        self.request_metrics.values().map(|m| m.failures).sum()
    }

    /// Returns true if this peer is blacklisted.
    pub fn is_blacklisted(&self) -> bool {
        self.is_blacklisted
    }

    /// Returns the compliance score.
    pub fn compliance_score(&self) -> f64 {
        self.compliance_score
    }

    /// Returns the decayed reliability.
    pub fn decayed_reliability(&self) -> f64 {
        self.decayed_reliability
    }

    /// Returns the time since this peer was first connected.
    pub fn connection_duration(&self) -> Duration {
        Instant::now().saturating_duration_since(self.connected_at)
    }

    /// Returns the metrics for a specific request type.
    pub fn metrics_for(&self, request_type: RequestType) -> Option<&RequestTypeMetrics> {
        self.request_metrics.get(&request_type)
    }

    /// Calculates the maximum concurrent requests allowed based on score.
    ///
    /// Higher scores allow more concurrent requests.
    pub fn max_concurrent_requests(&self) -> i64 {
        let score = self.compute_overall_score();
        (self.config.max_concurrent_base as f64 * score).max(1.0) as i64
    }

    /// Returns true if this peer can accept more requests given current load.
    pub fn can_accept_request(&self, current_requests: i64) -> bool {
        if self.is_blacklisted {
            return false;
        }
        current_requests < self.max_concurrent_requests()
    }

    /// Converts the peer score to the legacy i64 score format for compatibility.
    ///
    /// Maps the [0.0, 1.0] composite score to [-50, 50] range.
    pub fn to_legacy_score(&self) -> i64 {
        let score = self.compute_overall_score();
        // Map [0.0, 1.0] to [-50, 50]
        ((score - 0.5) * 100.0) as i64
    }

    /// Creates a PeerScore from a legacy i64 score for migration.
    pub fn from_legacy_score(legacy: i64, config: PeerScoringConfig) -> Self {
        let mut score = Self::new(config);

        // Map [-50, 50] to [0.0, 1.0]
        let normalized = (legacy as f64 + 50.0) / 100.0;
        score.decayed_reliability = normalized.clamp(0.0, 1.0);

        score
    }
}
