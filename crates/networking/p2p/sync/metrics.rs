//! Snap sync metrics and progress logging infrastructure.
//!
//! This module provides structured metrics collection and formatted progress logging
//! for the snap sync process. It tracks per-phase metrics, per-peer performance,
//! and generates human-readable progress reports.

use std::{
    collections::HashMap,
    sync::{
        RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use ethrex_common::H256;
use tracing::{debug, info};

/// Progress update interval for periodic status reports
pub const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_secs(2);

/// Phase numbers for snap sync
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SyncPhase {
    BlockHeaders = 1,
    AccountRanges = 2,
    AccountInsertion = 3,
    StorageRanges = 4,
    StorageInsertion = 5,
    StateHealing = 6,
    StorageHealing = 7,
    Bytecodes = 8,
    Validation = 9,
}

impl SyncPhase {
    pub fn name(&self) -> &'static str {
        match self {
            SyncPhase::BlockHeaders => "BLOCK HEADERS",
            SyncPhase::AccountRanges => "ACCOUNT RANGES",
            SyncPhase::AccountInsertion => "ACCOUNT INSERTION",
            SyncPhase::StorageRanges => "STORAGE RANGES",
            SyncPhase::StorageInsertion => "STORAGE INSERTION",
            SyncPhase::StateHealing => "STATE HEALING",
            SyncPhase::StorageHealing => "STORAGE HEALING",
            SyncPhase::Bytecodes => "BYTECODES",
            SyncPhase::Validation => "VALIDATION",
        }
    }

    pub fn number(&self) -> u8 {
        *self as u8
    }

    pub fn total_phases() -> u8 {
        8 // Not counting validation as it's optional
    }
}

/// Metrics for a single peer's performance
#[derive(Debug, Default)]
pub struct PeerMetrics {
    /// Total requests made to this peer
    pub requests: AtomicU64,
    /// Total bytes received from this peer
    pub bytes_received: AtomicU64,
    /// Sum of all request latencies in milliseconds
    pub total_latency_ms: AtomicU64,
    /// Number of failed requests
    pub failures: AtomicU64,
}

impl PeerMetrics {
    pub fn record_request(&self, bytes: u64, latency_ms: u64, success: bool) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
        self.total_latency_ms
            .fetch_add(latency_ms, Ordering::Relaxed);
        if !success {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn avg_latency_ms(&self) -> u64 {
        let requests = self.requests.load(Ordering::Relaxed);
        if requests == 0 {
            return 0;
        }
        self.total_latency_ms.load(Ordering::Relaxed) / requests
    }

    pub fn throughput_bytes_per_sec(&self, elapsed_secs: f64) -> f64 {
        if elapsed_secs <= 0.0 {
            return 0.0;
        }
        self.bytes_received.load(Ordering::Relaxed) as f64 / elapsed_secs
    }

    pub fn success_rate(&self) -> f64 {
        let requests = self.requests.load(Ordering::Relaxed);
        if requests == 0 {
            return 1.0;
        }
        let failures = self.failures.load(Ordering::Relaxed);
        (requests - failures) as f64 / requests as f64
    }
}

/// Summary of a completed phase
#[derive(Debug, Clone)]
pub struct PhaseSummary {
    pub duration: Duration,
    pub items_processed: u64,
    pub bytes_transferred: u64,
    pub extra_info: Option<String>,
}

impl PhaseSummary {
    pub fn rate_per_sec(&self) -> f64 {
        let secs = self.duration.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        self.items_processed as f64 / secs
    }

    pub fn throughput_mb_per_sec(&self) -> f64 {
        let secs = self.duration.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        (self.bytes_transferred as f64 / 1_048_576.0) / secs
    }
}

/// Main sync metrics tracker
#[derive(Debug)]
pub struct SyncMetrics {
    /// When the sync started
    pub sync_start: Instant,
    /// Current phase start time
    pub phase_start: RwLock<Option<Instant>>,
    /// Current phase
    pub current_phase: RwLock<Option<SyncPhase>>,
    /// Total bytes received across all phases
    pub total_bytes_received: AtomicU64,
    /// Per-peer metrics
    pub peer_metrics: RwLock<HashMap<H256, PeerMetrics>>,
    /// Phase summaries for final report
    pub phase_summaries: RwLock<HashMap<SyncPhase, PhaseSummary>>,
    /// Last progress update time
    pub last_progress_update: RwLock<Instant>,
}

impl Default for SyncMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncMetrics {
    pub fn new() -> Self {
        Self {
            sync_start: Instant::now(),
            phase_start: RwLock::new(None),
            current_phase: RwLock::new(None),
            total_bytes_received: AtomicU64::new(0),
            peer_metrics: RwLock::new(HashMap::new()),
            phase_summaries: RwLock::new(HashMap::new()),
            last_progress_update: RwLock::new(Instant::now()),
        }
    }

    /// Record bytes received from a peer
    pub fn record_peer_request(&self, peer_id: H256, bytes: u64, latency_ms: u64, success: bool) {
        self.total_bytes_received
            .fetch_add(bytes, Ordering::Relaxed);

        if let Ok(mut metrics) = self.peer_metrics.write() {
            metrics
                .entry(peer_id)
                .or_default()
                .record_request(bytes, latency_ms, success);
        }
    }

    /// Get or create peer metrics
    pub fn get_peer_metrics(&self, peer_id: &H256) -> Option<(u64, u64, u64, f64)> {
        let metrics = self.peer_metrics.read().ok()?;
        metrics.get(peer_id).map(|m| {
            (
                m.requests.load(Ordering::Relaxed),
                m.bytes_received.load(Ordering::Relaxed),
                m.avg_latency_ms(),
                m.success_rate(),
            )
        })
    }

    /// Check if we should emit a progress update
    pub fn should_update_progress(&self) -> bool {
        self.last_progress_update
            .read()
            .map(|guard| guard.elapsed() >= PROGRESS_UPDATE_INTERVAL)
            .unwrap_or(true)
    }

    /// Mark that we've emitted a progress update
    pub fn mark_progress_updated(&self) {
        if let Ok(mut guard) = self.last_progress_update.write() {
            *guard = Instant::now();
        }
    }

    /// Get elapsed time since sync start
    pub fn elapsed(&self) -> Duration {
        self.sync_start.elapsed()
    }

    /// Get elapsed time since current phase start
    pub fn phase_elapsed(&self) -> Duration {
        self.phase_start
            .read()
            .ok()
            .and_then(|guard| *guard)
            .map(|start| start.elapsed())
            .unwrap_or_default()
    }

    /// Store a phase summary
    pub fn record_phase_summary(&self, phase: SyncPhase, summary: PhaseSummary) {
        if let Ok(mut summaries) = self.phase_summaries.write() {
            summaries.insert(phase, summary);
        }
    }

    /// Get total bytes as formatted string
    pub fn total_bytes_formatted(&self) -> String {
        format_bytes(self.total_bytes_received.load(Ordering::Relaxed))
    }

    /// Get best and worst performing peers
    pub fn get_peer_rankings(
        &self,
        elapsed_secs: f64,
    ) -> (Option<PeerRanking>, Option<PeerRanking>) {
        let Ok(metrics) = self.peer_metrics.read() else {
            return (None, None);
        };
        if metrics.is_empty() {
            return (None, None);
        }

        let mut rankings: Vec<_> = metrics
            .iter()
            .map(|(id, m)| PeerRanking {
                peer_id: *id,
                throughput: m.throughput_bytes_per_sec(elapsed_secs),
                avg_latency_ms: m.avg_latency_ms(),
                failures: m.failures.load(Ordering::Relaxed),
            })
            .collect();

        rankings.sort_by(|a, b| {
            b.throughput
                .partial_cmp(&a.throughput)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let best = rankings.first().cloned();
        let worst = rankings.last().cloned();

        (best, worst)
    }

    /// Get total peer count
    pub fn peer_count(&self) -> usize {
        self.peer_metrics.read().map(|m| m.len()).unwrap_or(0)
    }
}

/// Ranking info for a peer
#[derive(Debug, Clone)]
pub struct PeerRanking {
    pub peer_id: H256,
    pub throughput: f64,
    pub avg_latency_ms: u64,
    pub failures: u64,
}

/// Phase logger for consistent formatting
pub struct PhaseLogger;

impl PhaseLogger {
    /// Log phase start with separator
    pub fn phase_start(phase: SyncPhase) {
        let phase_num = phase.number();
        let total = SyncPhase::total_phases();
        let name = phase.name();

        info!("");
        info!(
            "━━━ PHASE {phase_num}/{total}: {name} ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        );
    }

    /// Log phase completion with summary
    pub fn phase_end(phase: SyncPhase, summary: &PhaseSummary) {
        let name = phase.name().to_lowercase();
        let duration = format_duration(summary.duration);
        let items = summary.items_processed;
        let rate = summary.rate_per_sec();
        let bytes = format_bytes(summary.bytes_transferred);

        let extra = summary
            .extra_info
            .as_ref()
            .map(|s| format!("\n      {s}"))
            .unwrap_or_default();

        info!("✓ {name} complete: {items} items in {duration} ({rate:.1}/s, {bytes}){extra}");
    }

    /// Log sync start banner
    pub fn sync_start(
        target_hash: H256,
        target_block: u64,
        current_hash: H256,
        current_block: u64,
    ) {
        let gap = target_block.saturating_sub(current_block);

        info!("╭──────────────────────────────────────────────────────────────────────────────╮");
        info!("│ SNAP SYNC STARTED                                                            │");
        info!(
            target_hash = %format_hash_short(&target_hash),
            target_block,
            "Target block"
        );
        info!(
            current_hash = %format_hash_short(&current_hash),
            current_block,
            "Current block"
        );
        info!(gap, "Blocks to sync");
        info!("╰──────────────────────────────────────────────────────────────────────────────╯");
    }

    /// Log sync completion summary
    pub fn sync_complete(metrics: &SyncMetrics) {
        let total_time = format_duration(metrics.elapsed());
        let total_data = metrics.total_bytes_formatted();
        let elapsed_secs = metrics.elapsed().as_secs_f64();
        let avg_throughput = if elapsed_secs > 0.0 {
            metrics.total_bytes_received.load(Ordering::Relaxed) as f64 / elapsed_secs / 1_048_576.0
        } else {
            0.0
        };

        info!("╭──────────────────────────────────────────────────────────────────────────────╮");
        info!("│ SNAP SYNC COMPLETE                                                           │");
        info!("├──────────────────────────────────────────────────────────────────────────────┤");
        info!(
            "│ Total time:      {total_time}                                                   │"
        );
        info!(
            "│ Data downloaded: {total_data}                                                     │"
        );
        info!(
            "│ Avg throughput:  {avg_throughput:.1} MB/s                                                    │"
        );
        info!("├──────────────────────────────────────────────────────────────────────────────┤");

        // Phase breakdown
        info!("│ Phase breakdown:                                                             │");
        let summaries = metrics
            .phase_summaries
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for phase in [
            SyncPhase::BlockHeaders,
            SyncPhase::AccountRanges,
            SyncPhase::AccountInsertion,
            SyncPhase::StorageRanges,
            SyncPhase::StorageInsertion,
            SyncPhase::StateHealing,
            SyncPhase::StorageHealing,
            SyncPhase::Bytecodes,
        ] {
            if let Some(summary) = summaries.get(&phase) {
                let pct = if elapsed_secs > 0.0 {
                    summary.duration.as_secs_f64() / elapsed_secs * 100.0
                } else {
                    0.0
                };
                let duration = format_duration(summary.duration);
                let name = phase.name().to_lowercase();
                info!(
                    "│   {}: {duration} ({pct:.1}%)                                        │",
                    phase.number()
                );
                debug!(
                    "│     -> {name}                                                            │"
                );
            }
        }

        info!("├──────────────────────────────────────────────────────────────────────────────┤");

        // Peer performance
        let (best, worst) = metrics.get_peer_rankings(elapsed_secs);
        info!("│ Peer performance:                                                            │");
        if let Some(best) = best {
            info!(
                peer_id = %format_hash_short(&best.peer_id),
                throughput_mb = best.throughput / 1_048_576.0,
                latency_ms = best.avg_latency_ms,
                failures = best.failures,
                "Best peer"
            );
        }
        if let Some(worst) = worst {
            info!(
                peer_id = %format_hash_short(&worst.peer_id),
                throughput_mb = worst.throughput / 1_048_576.0,
                latency_ms = worst.avg_latency_ms,
                failures = worst.failures,
                "Worst peer"
            );
        }
        info!(total_peers = metrics.peer_count(), "Peers used");

        info!("╰──────────────────────────────────────────────────────────────────────────────╯");
    }

    /// Log a progress update for phases with known totals (progress bar)
    pub fn progress_with_total(
        current: u64,
        total: u64,
        elapsed: Duration,
        extra_metrics: &[(&str, String)],
    ) {
        let pct = if total > 0 {
            (current as f64 / total as f64 * 100.0).min(100.0)
        } else {
            0.0
        };
        let rate = if elapsed.as_secs_f64() > 0.0 {
            current as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        let bar_width = 40;
        let filled = (pct / 100.0 * bar_width as f64) as usize;
        let bar: String = "▓".repeat(filled) + &"░".repeat(bar_width - filled);

        debug!("  PROGRESS {bar} {pct:.1}%");
        debug!(
            "  Items: {current} / {total}  │  Rate: {rate:.1}/s  │  Elapsed: {}",
            format_duration(elapsed)
        );

        for (label, value) in extra_metrics {
            debug!("  {label}: {value}");
        }
    }

    /// Log a progress update for phases without known totals (keyspace indicator)
    pub fn progress_keyspace(
        current_position: &H256,
        items_fetched: u64,
        bytes_received: u64,
        elapsed: Duration,
        extra_metrics: &[(&str, String)],
    ) {
        let rate = if elapsed.as_secs_f64() > 0.0 {
            items_fetched as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };
        let throughput = if elapsed.as_secs_f64() > 0.0 {
            bytes_received as f64 / elapsed.as_secs_f64() / 1_048_576.0
        } else {
            0.0
        };

        // Show keyspace position as first byte
        let pos_byte = current_position.0[0];

        debug!("  KEYSPACE   0x00 ━━━━━━━━━━━━━━━━━━━━━━━━━━▶ 0x{pos_byte:02x}... ░░░░░░░░ 0xff");
        debug!(
            "  Items fetched: {items_fetched}  │  Rate: {rate:.1}/s  │  Throughput: {throughput:.1} MB/s"
        );
        debug!(
            "  Elapsed: {}  │  Data: {}",
            format_duration(elapsed),
            format_bytes(bytes_received)
        );

        for (label, value) in extra_metrics {
            debug!("  {label}: {value}");
        }
    }

    /// Log peer table status
    pub fn peer_table(peers: &[(H256, u64, f64, f64)]) {
        if peers.is_empty() {
            return;
        }

        debug!(active_peers = peers.len(), "Peer table status");
        for (peer_id, avg_latency, throughput_mbps, success_rate) in peers.iter().take(5) {
            let status = if *success_rate > 0.9 {
                "ok"
            } else if *success_rate > 0.7 {
                "slow"
            } else {
                "poor"
            };
            debug!(
                peer_id = %format_hash_short(peer_id),
                avg_latency_ms = avg_latency,
                throughput_mbps = %format!("{throughput_mbps:.1}"),
                status,
                "Peer stats"
            );
        }
        if peers.len() > 5 {
            debug!(
                additional_peers = peers.len() - 5,
                "Additional peers not shown"
            );
        }
    }

    /// Log a retry attempt
    #[allow(dead_code)]
    pub fn retry(
        attempt: u32,
        max_attempts: u32,
        peer_id: H256,
        reason: &str,
        next_peer: Option<H256>,
    ) {
        if let Some(next) = next_peer {
            debug!(
                attempt,
                max_attempts,
                peer_id = %format_hash_short(&peer_id),
                reason,
                next_peer = %format_hash_short(&next),
                "Retry attempt with fallback peer"
            );
        } else {
            debug!(
                attempt,
                max_attempts,
                peer_id = %format_hash_short(&peer_id),
                reason,
                "Retry attempt"
            );
        }
    }
}

/// Format a H256 hash as a short hex string (first 8 characters)
pub fn format_hash_short(hash: &H256) -> String {
    format!(
        "0x{:02x}{:02x}{:02x}{:02x}...",
        hash.0[0], hash.0[1], hash.0[2], hash.0[3]
    )
}

/// Format bytes as human-readable string
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format duration as human-readable string
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{hours}h {mins}m")
    } else if secs >= 60 {
        let mins = secs / 60;
        let secs = secs % 60;
        format!("{mins}m {secs}s")
    } else if secs > 0 {
        let millis = d.subsec_millis();
        format!("{secs}.{millis:03}s")
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// Format a rate as items per second
pub fn format_rate(items: u64, duration: Duration) -> String {
    let secs = duration.as_secs_f64();
    if secs <= 0.0 {
        return "0/s".to_string();
    }
    let rate = items as f64 / secs;
    if rate >= 1_000_000.0 {
        format!("{:.1}M/s", rate / 1_000_000.0)
    } else if rate >= 1_000.0 {
        format!("{:.1}k/s", rate / 1_000.0)
    } else {
        format!("{rate:.1}/s")
    }
}

// Hash implementation for SyncPhase to use in HashMap
impl std::hash::Hash for SyncPhase {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (*self as u8).hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(1572864), "1.5 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(format_duration(Duration::from_secs(5)), "5.000s");
        assert_eq!(format_duration(Duration::from_secs(65)), "1m 5s");
        assert_eq!(format_duration(Duration::from_secs(3665)), "1h 1m");
    }

    #[test]
    fn test_peer_metrics() {
        let metrics = PeerMetrics::default();
        metrics.record_request(1000, 50, true);
        metrics.record_request(2000, 100, true);
        metrics.record_request(500, 25, false);

        assert_eq!(metrics.requests.load(Ordering::Relaxed), 3);
        assert_eq!(metrics.bytes_received.load(Ordering::Relaxed), 3500);
        assert_eq!(metrics.avg_latency_ms(), 58); // (50+100+25)/3
        assert_eq!(metrics.failures.load(Ordering::Relaxed), 1);
        assert!((metrics.success_rate() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_phase_summary() {
        let summary = PhaseSummary {
            duration: Duration::from_secs(10),
            items_processed: 1000,
            bytes_transferred: 10_485_760, // 10 MB
            extra_info: None,
        };

        assert!((summary.rate_per_sec() - 100.0).abs() < 0.01);
        assert!((summary.throughput_mb_per_sec() - 1.0).abs() < 0.01);
    }
}
