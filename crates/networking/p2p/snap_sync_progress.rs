//! Snap sync progress tracking with phase table display.
//!
//! Tracks progress across 8 phases with timing, counts, and a formatted table output.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use tracing::info;

/// Total number of phases in snap sync
pub const TOTAL_PHASES: u8 = 8;

/// Phase names for display
const PHASE_NAMES: [&str; 8] = [
    "Header Download",
    "Account Download",
    "Account Insertion",
    "Storage Download",
    "Storage Insertion",
    "State Healing",
    "Storage Healing",
    "Bytecode Download",
];

/// Global progress tracker
pub static SNAP_PROGRESS: once_cell::sync::Lazy<SnapSyncProgress> =
    once_cell::sync::Lazy::new(SnapSyncProgress::new);

/// Progress tracker for a single phase
#[derive(Debug)]
pub struct PhaseProgress {
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
    pub current: AtomicU64,
    pub total: AtomicU64,
}

impl PhaseProgress {
    pub fn new() -> Self {
        Self {
            start_time: None,
            end_time: None,
            current: AtomicU64::new(0),
            total: AtomicU64::new(0),
        }
    }

    pub fn start(&mut self) {
        self.start_time = Some(Instant::now());
        self.end_time = None;
    }

    pub fn finish(&mut self) {
        self.end_time = Some(Instant::now());
    }

    pub fn elapsed(&self) -> Duration {
        match (self.start_time, self.end_time) {
            (Some(start), Some(end)) => end.duration_since(start),
            (Some(start), None) => start.elapsed(),
            _ => Duration::ZERO,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.end_time.is_some()
    }
}

impl Default for PhaseProgress {
    fn default() -> Self {
        Self::new()
    }
}

/// Main progress tracker for snap sync
pub struct SnapSyncProgress {
    pub phases: Mutex<[PhaseProgress; 8]>,
    pub sync_start: Mutex<Option<Instant>>,
    pub last_table_print: Mutex<Instant>,
    /// Interval between table prints
    pub table_interval: Duration,
}

impl SnapSyncProgress {
    pub fn new() -> Self {
        Self {
            phases: Mutex::new(std::array::from_fn(|_| PhaseProgress::new())),
            sync_start: Mutex::new(None),
            last_table_print: Mutex::new(Instant::now()),
            table_interval: Duration::from_secs(30),
        }
    }

    /// Start tracking a phase
    pub fn start_phase(&self, phase: usize) {
        if phase < 8 {
            let mut phases = self.phases.lock();
            phases[phase].start();

            // Set sync start time on first phase
            let mut sync_start = self.sync_start.lock();
            if sync_start.is_none() {
                *sync_start = Some(Instant::now());
            }
        }
    }

    /// Mark a phase as complete
    pub fn finish_phase(&self, phase: usize) {
        if phase < 8 {
            let mut phases = self.phases.lock();
            phases[phase].finish();
        }
        self.print_table();
    }

    /// Update progress for a phase
    pub fn update_progress(&self, phase: usize, current: u64, total: u64) {
        if phase < 8 {
            let phases = self.phases.lock();
            phases[phase].current.store(current, Ordering::Relaxed);
            phases[phase].total.store(total, Ordering::Relaxed);
        }
    }

    /// Increment current count for a phase
    pub fn increment(&self, phase: usize, delta: u64) {
        if phase < 8 {
            let phases = self.phases.lock();
            phases[phase].current.fetch_add(delta, Ordering::Relaxed);
        }
    }

    /// Set total for a phase
    pub fn set_total(&self, phase: usize, total: u64) {
        if phase < 8 {
            let phases = self.phases.lock();
            phases[phase].total.store(total, Ordering::Relaxed);
        }
    }

    /// Maybe print table if interval has passed
    pub fn maybe_print_table(&self) {
        let mut last_print = self.last_table_print.lock();
        if last_print.elapsed() >= self.table_interval {
            *last_print = Instant::now();
            drop(last_print);
            self.print_table();
        }
    }

    /// Print the progress table
    pub fn print_table(&self) {
        let phases = self.phases.lock();
        let sync_start = self.sync_start.lock();

        let total_elapsed = sync_start.map(|s| s.elapsed()).unwrap_or(Duration::ZERO);

        let mut table = String::new();
        table.push_str("\n┌─────────────────────────────────────────────────────────────────┐\n");
        table.push_str("│                    SNAP SYNC PROGRESS                           │\n");
        table.push_str("├────┬────────────────────┬────────────────┬──────────┬───────────┤\n");
        table.push_str("│ #  │ Phase              │ Progress       │ Duration │ Status    │\n");
        table.push_str("├────┼────────────────────┼────────────────┼──────────┼───────────┤\n");

        for (i, phase) in phases.iter().enumerate() {
            let current = phase.current.load(Ordering::Relaxed);
            let total = phase.total.load(Ordering::Relaxed);
            let elapsed = phase.elapsed();

            let status = if phase.is_complete() {
                "✓ Done"
            } else if phase.start_time.is_some() {
                "► Active"
            } else {
                "○ Pending"
            };

            let progress = if total > 0 {
                format!("{} / {}", format_count(current), format_count(total))
            } else if current > 0 {
                format_count(current)
            } else {
                "-".to_string()
            };

            let duration = if elapsed > Duration::ZERO {
                format_duration(elapsed)
            } else {
                "-".to_string()
            };

            table.push_str(&format!(
                "│ {:>2} │ {:<18} │ {:>14} │ {:>8} │ {:<9} │\n",
                i + 1,
                PHASE_NAMES[i],
                progress,
                duration,
                status
            ));
        }

        table.push_str("├────┴────────────────────┴────────────────┴──────────┴───────────┤\n");
        table.push_str(&format!(
            "│ Total elapsed: {:<49} │\n",
            format_duration(total_elapsed)
        ));
        table.push_str("└─────────────────────────────────────────────────────────────────┘");

        info!("{}", table);
    }

    /// Reset all progress (for new sync)
    pub fn reset(&self) {
        let mut phases = self.phases.lock();
        for phase in phases.iter_mut() {
            *phase = PhaseProgress::new();
        }
        *self.sync_start.lock() = None;
    }
}

impl Default for SnapSyncProgress {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a large number with K/M/B suffix
pub fn format_count(count: u64) -> String {
    if count >= 1_000_000_000 {
        format!("{:.2}B", count as f64 / 1_000_000_000.0)
    } else if count >= 1_000_000 {
        format!("{:.2}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        format!("{}", count)
    }
}

/// Format duration in human-readable form
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    if total_secs >= 3600 {
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        format!("{}h {}m", hours, mins)
    } else if total_secs >= 60 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", total_secs)
    }
}

/// Calculate ETA based on progress
pub fn calculate_eta(current: u64, total: u64, elapsed: Duration) -> Option<Duration> {
    if current == 0 || total == 0 || current >= total {
        return None;
    }
    let rate = current as f64 / elapsed.as_secs_f64();
    if rate <= 0.0 {
        return None;
    }
    let remaining = (total - current) as f64 / rate;
    Some(Duration::from_secs_f64(remaining))
}

/// Format rate per second
pub fn format_rate(rate: f64) -> String {
    if rate >= 1_000_000.0 {
        format!("{:.1}M/s", rate / 1_000_000.0)
    } else if rate >= 1_000.0 {
        format!("{:.1}K/s", rate / 1_000.0)
    } else {
        format!("{:.0}/s", rate)
    }
}
