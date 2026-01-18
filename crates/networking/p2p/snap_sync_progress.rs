//! Snap sync progress tracking and logging.
//!
//! Provides clear, consistent progress logging for snap sync with:
//! - Phase indicators (e.g., "[SNAP SYNC] Phase 3/9: Downloading Accounts")
//! - Progress within phases (e.g., "12.5M / 18M accounts (69%)")
//! - Download rates and ETA calculations
//! - Summary statistics at phase completion

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::info;

/// Total number of phases in snap sync (excluding NotStarted and Completed)
pub const TOTAL_PHASES: u8 = 8;

/// Phase names for display
const PHASE_NAMES: [&str; 10] = [
    "Not Started",
    "Downloading Headers",
    "Downloading Accounts",
    "Inserting Accounts",
    "Downloading Storage",
    "Inserting Storage",
    "Healing State",
    "Healing Storage",
    "Downloading Bytecodes",
    "Completed",
];

/// Get the display phase number (1-8 for active phases)
fn get_phase_number(phase: u8) -> u8 {
    match phase {
        0 => 0,  // NotStarted
        1 => 1,  // HeaderDownload
        2 => 2,  // AccountDownload
        3 => 3,  // AccountInsertion
        4 => 4,  // StorageDownload
        5 => 5,  // StorageInsertion
        6 => 6,  // StateHealing
        7 => 7,  // StorageHealing (shown as part of healing)
        8 => 8,  // BytecodeDownload
        9 => 8,  // Completed (same as last phase)
        _ => 0,
    }
}

/// Format a large number with appropriate suffix (K, M, B)
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

/// Calculate ETA based on current progress
pub fn calculate_eta(current: u64, total: u64, elapsed: Duration) -> Option<Duration> {
    if current == 0 || total == 0 || current >= total {
        return None;
    }
    let rate = current as f64 / elapsed.as_secs_f64();
    if rate <= 0.0 {
        return None;
    }
    let remaining = total - current;
    let eta_secs = remaining as f64 / rate;
    if eta_secs.is_finite() && eta_secs > 0.0 {
        Some(Duration::from_secs_f64(eta_secs))
    } else {
        None
    }
}

/// Progress tracker for a single phase
#[derive(Debug)]
pub struct PhaseProgress {
    pub current: AtomicU64,
    pub total: AtomicU64,
    pub start_time: Mutex<Option<Instant>>,
    pub last_log_time: Mutex<Option<Instant>>,
}

impl Default for PhaseProgress {
    fn default() -> Self {
        Self {
            current: AtomicU64::new(0),
            total: AtomicU64::new(0),
            start_time: Mutex::new(None),
            last_log_time: Mutex::new(None),
        }
    }
}

impl PhaseProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(&self, total: u64) {
        self.current.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        *self.start_time.lock().await = Some(Instant::now());
        *self.last_log_time.lock().await = None;
    }

    pub async fn reset(&self) {
        self.current.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        *self.start_time.lock().await = None;
        *self.last_log_time.lock().await = None;
    }

    pub fn increment(&self, amount: u64) {
        self.current.fetch_add(amount, Ordering::Relaxed);
    }

    pub fn set_current(&self, value: u64) {
        self.current.store(value, Ordering::Relaxed);
    }

    pub fn set_total(&self, value: u64) {
        self.total.store(value, Ordering::Relaxed);
    }

    pub fn get_current(&self) -> u64 {
        self.current.load(Ordering::Relaxed)
    }

    pub fn get_total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    pub async fn elapsed(&self) -> Duration {
        self.start_time
            .lock()
            .await
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    pub async fn rate(&self) -> f64 {
        let elapsed = self.elapsed().await;
        let current = self.get_current();
        if elapsed.as_secs_f64() > 0.0 {
            current as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        }
    }

    pub async fn eta(&self) -> Option<Duration> {
        let elapsed = self.elapsed().await;
        calculate_eta(self.get_current(), self.get_total(), elapsed)
    }

    pub fn percentage(&self) -> f64 {
        let total = self.get_total();
        if total == 0 {
            0.0
        } else {
            (self.get_current() as f64 / total as f64) * 100.0
        }
    }
}

/// Central progress tracker for the entire snap sync process
#[derive(Debug)]
pub struct SnapSyncProgress {
    /// Overall sync start time
    pub sync_start: Mutex<Option<Instant>>,
    /// Current phase (as u8 matching SnapSyncPhase)
    pub current_phase: AtomicU64,
    /// Pivot block number
    pub pivot_block: AtomicU64,
    /// Progress for headers
    pub headers: PhaseProgress,
    /// Progress for accounts
    pub accounts: PhaseProgress,
    /// Progress for storage slots
    pub storage: PhaseProgress,
    /// Progress for bytecodes
    pub bytecodes: PhaseProgress,
    /// Progress for healing (nodes healed)
    pub healing: PhaseProgress,
    /// Minimum interval between progress logs
    pub log_interval: Duration,
}

impl Default for SnapSyncProgress {
    fn default() -> Self {
        Self {
            sync_start: Mutex::new(None),
            current_phase: AtomicU64::new(0),
            pivot_block: AtomicU64::new(0),
            headers: PhaseProgress::new(),
            accounts: PhaseProgress::new(),
            storage: PhaseProgress::new(),
            bytecodes: PhaseProgress::new(),
            healing: PhaseProgress::new(),
            log_interval: Duration::from_secs(5),
        }
    }
}

impl SnapSyncProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the overall sync
    pub async fn start_sync(&self, pivot_block: u64) {
        *self.sync_start.lock().await = Some(Instant::now());
        self.pivot_block.store(pivot_block, Ordering::Relaxed);
        self.current_phase.store(0, Ordering::Relaxed);
        info!(
            "================================================================================\n\
             [SNAP SYNC] Starting snap sync to pivot block #{}\n\
             ================================================================================",
            pivot_block
        );
    }

    /// Log entry into a new phase
    pub async fn enter_phase(&self, phase: u8) {
        self.current_phase.store(phase as u64, Ordering::Relaxed);
        let phase_num = get_phase_number(phase);
        let phase_name = PHASE_NAMES.get(phase as usize).unwrap_or(&"Unknown");

        info!(
            "--------------------------------------------------------------------------------\n\
             [SNAP SYNC] Phase {}/{}: {}\n\
             --------------------------------------------------------------------------------",
            phase_num, TOTAL_PHASES, phase_name
        );
    }

    /// Log completion of a phase with summary
    pub async fn complete_phase(&self, phase: u8) {
        let phase_num = get_phase_number(phase);
        let phase_name = PHASE_NAMES.get(phase as usize).unwrap_or(&"Unknown");

        // Get the appropriate progress tracker for this phase
        let (elapsed, processed, rate) = match phase {
            1 => {
                // Headers
                let elapsed = self.headers.elapsed().await;
                let processed = self.headers.get_current();
                let rate = self.headers.rate().await;
                (elapsed, processed, rate)
            }
            2 | 3 => {
                // Account download/insertion
                let elapsed = self.accounts.elapsed().await;
                let processed = self.accounts.get_current();
                let rate = self.accounts.rate().await;
                (elapsed, processed, rate)
            }
            4 | 5 => {
                // Storage download/insertion
                let elapsed = self.storage.elapsed().await;
                let processed = self.storage.get_current();
                let rate = self.storage.rate().await;
                (elapsed, processed, rate)
            }
            6 | 7 => {
                // Healing
                let elapsed = self.healing.elapsed().await;
                let processed = self.healing.get_current();
                let rate = self.healing.rate().await;
                (elapsed, processed, rate)
            }
            8 => {
                // Bytecodes
                let elapsed = self.bytecodes.elapsed().await;
                let processed = self.bytecodes.get_current();
                let rate = self.bytecodes.rate().await;
                (elapsed, processed, rate)
            }
            _ => (Duration::ZERO, 0, 0.0),
        };

        info!(
            "[SNAP SYNC] Phase {}/{} complete: {} | Processed: {} | Rate: {} | Duration: {}",
            phase_num,
            TOTAL_PHASES,
            phase_name,
            format_count(processed),
            format_rate(rate),
            format_duration(elapsed)
        );

        // Print progress table after each phase
        self.print_progress_table().await;
    }

    /// Log overall sync completion
    pub async fn complete_sync(&self) {
        let total_elapsed = self
            .sync_start
            .lock()
            .await
            .map(|t| t.elapsed())
            .unwrap_or_default();

        let pivot = self.pivot_block.load(Ordering::Relaxed);
        let headers = self.headers.get_current();
        let accounts = self.accounts.get_current();
        let storage = self.storage.get_current();
        let bytecodes = self.bytecodes.get_current();

        info!(
            "================================================================================\n\
             [SNAP SYNC] COMPLETED SUCCESSFULLY\n\
             ================================================================================\n\
             Pivot Block:    #{}\n\
             Total Duration: {}\n\
             \n\
             Summary:\n\
             - Headers:    {}\n\
             - Accounts:   {}\n\
             - Storage:    {} slots\n\
             - Bytecodes:  {}\n\
             ================================================================================",
            pivot,
            format_duration(total_elapsed),
            format_count(headers),
            format_count(accounts),
            format_count(storage),
            format_count(bytecodes),
        );

        // Print final progress table
        self.print_progress_table().await;
    }

    /// Print a progress summary table showing all phases
    pub async fn print_progress_table(&self) {
        let current_phase = self.current_phase.load(Ordering::Relaxed) as u8;

        // Collect phase data
        let phases: [(u8, &str, &PhaseProgress); 5] = [
            (1, "Headers", &self.headers),
            (2, "Accounts", &self.accounts),
            (4, "Storage", &self.storage),
            (6, "Healing", &self.healing),
            (8, "Bytecodes", &self.bytecodes),
        ];

        let mut table = String::new();
        table.push_str("\n┌────────────────────────────────────────────────────────────────────────────┐\n");
        table.push_str("│                        SNAP SYNC PROGRESS SUMMARY                          │\n");
        table.push_str("├─────────┬──────────────────┬────────────┬─────────────┬───────────────────┤\n");
        table.push_str("│ Phase   │ Description      │ Processed  │ Rate        │ Duration          │\n");
        table.push_str("├─────────┼──────────────────┼────────────┼─────────────┼───────────────────┤\n");

        for (phase_num, name, progress) in phases {
            let processed = progress.get_current();
            let elapsed = progress.elapsed().await;
            let rate = progress.rate().await;

            let status = if current_phase > phase_num + 1 || (current_phase == 9 && phase_num == 8) {
                "✓"
            } else if current_phase >= phase_num && current_phase <= phase_num + 1 {
                "→"
            } else {
                " "
            };

            let processed_str = if processed > 0 {
                format_count(processed)
            } else {
                "-".to_string()
            };

            let rate_str = if rate > 0.0 {
                format_rate(rate)
            } else {
                "-".to_string()
            };

            let duration_str = if elapsed.as_secs() > 0 {
                format_duration(elapsed)
            } else {
                "-".to_string()
            };

            table.push_str(&format!(
                "│ {} {}/8   │ {:16} │ {:>10} │ {:>11} │ {:>17} │\n",
                status, phase_num, name, processed_str, rate_str, duration_str
            ));
        }

        table.push_str("└─────────┴──────────────────┴────────────┴─────────────┴───────────────────┘");

        info!("{}", table);
    }

    /// Log progress for headers download
    pub async fn log_headers_progress(&self, downloaded: u64, total: u64) {
        self.headers.set_current(downloaded);
        self.headers.set_total(total);

        if !self.should_log(&self.headers).await {
            return;
        }

        let rate = self.headers.rate().await;
        let eta = self.headers.eta().await;
        let pct = self.headers.percentage();

        let eta_str = eta.map(format_duration).unwrap_or_else(|| "calculating...".to_string());

        info!(
            "[SNAP SYNC] Phase 1/{}: Headers: {} / {} ({:.1}%) | Rate: {} | ETA: {}",
            TOTAL_PHASES,
            format_count(downloaded),
            format_count(total),
            pct,
            format_rate(rate),
            eta_str
        );

        self.mark_logged(&self.headers).await;
    }

    /// Log progress for account download
    pub async fn log_accounts_download_progress(&self, downloaded: u64, rate_per_sec: Option<f64>) {
        self.accounts.set_current(downloaded);

        if !self.should_log(&self.accounts).await {
            return;
        }

        let elapsed = self.accounts.elapsed().await;
        let rate = rate_per_sec.unwrap_or_else(|| {
            if elapsed.as_secs_f64() > 0.0 {
                downloaded as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            }
        });

        info!(
            "[SNAP SYNC] Phase 2/{}: Downloading accounts: {} | Rate: {} | Elapsed: {}",
            TOTAL_PHASES,
            format_count(downloaded),
            format_rate(rate),
            format_duration(elapsed)
        );

        self.mark_logged(&self.accounts).await;
    }

    /// Log progress for account insertion
    pub async fn log_accounts_insertion_progress(&self, inserted: u64, total: u64) {
        self.accounts.set_current(inserted);
        self.accounts.set_total(total);

        if !self.should_log(&self.accounts).await {
            return;
        }

        let rate = self.accounts.rate().await;
        let eta = self.accounts.eta().await;
        let pct = self.accounts.percentage();

        let eta_str = eta.map(format_duration).unwrap_or_else(|| "calculating...".to_string());

        info!(
            "[SNAP SYNC] Phase 3/{}: Inserting accounts: {} / {} ({:.1}%) | Rate: {} | ETA: {}",
            TOTAL_PHASES,
            format_count(inserted),
            format_count(total),
            pct,
            format_rate(rate),
            eta_str
        );

        self.mark_logged(&self.accounts).await;
    }

    /// Log progress for storage download
    pub async fn log_storage_download_progress(&self, downloaded_slots: u64, accounts_remaining: u64) {
        self.storage.set_current(downloaded_slots);

        if !self.should_log(&self.storage).await {
            return;
        }

        let elapsed = self.storage.elapsed().await;
        let rate = self.storage.rate().await;

        info!(
            "[SNAP SYNC] Phase 4/{}: Downloading storage: {} slots | {} accounts remaining | Rate: {} | Elapsed: {}",
            TOTAL_PHASES,
            format_count(downloaded_slots),
            format_count(accounts_remaining),
            format_rate(rate),
            format_duration(elapsed)
        );

        self.mark_logged(&self.storage).await;
    }

    /// Log progress for storage insertion
    pub async fn log_storage_insertion_progress(&self, inserted: u64, total: u64) {
        self.storage.set_current(inserted);
        self.storage.set_total(total);

        if !self.should_log(&self.storage).await {
            return;
        }

        let rate = self.storage.rate().await;
        let eta = self.storage.eta().await;
        let pct = self.storage.percentage();

        let eta_str = eta.map(format_duration).unwrap_or_else(|| "calculating...".to_string());

        info!(
            "[SNAP SYNC] Phase 5/{}: Inserting storage: {} / {} slots ({:.1}%) | Rate: {} | ETA: {}",
            TOTAL_PHASES,
            format_count(inserted),
            format_count(total),
            pct,
            format_rate(rate),
            eta_str
        );

        self.mark_logged(&self.storage).await;
    }

    /// Log progress for healing
    pub async fn log_healing_progress(&self, nodes_healed: u64, phase_name: &str) {
        self.healing.set_current(nodes_healed);

        if !self.should_log(&self.healing).await {
            return;
        }

        let elapsed = self.healing.elapsed().await;
        let rate = self.healing.rate().await;

        info!(
            "[SNAP SYNC] Phase 6/{}: {}: {} nodes healed | Rate: {} | Elapsed: {}",
            TOTAL_PHASES,
            phase_name,
            format_count(nodes_healed),
            format_rate(rate),
            format_duration(elapsed)
        );

        self.mark_logged(&self.healing).await;
    }

    /// Log progress for bytecode download
    pub async fn log_bytecodes_progress(&self, downloaded: u64, total: u64) {
        self.bytecodes.set_current(downloaded);
        self.bytecodes.set_total(total);

        if !self.should_log(&self.bytecodes).await {
            return;
        }

        let rate = self.bytecodes.rate().await;
        let eta = self.bytecodes.eta().await;
        let pct = self.bytecodes.percentage();

        let eta_str = eta.map(format_duration).unwrap_or_else(|| "calculating...".to_string());

        info!(
            "[SNAP SYNC] Phase 8/{}: Downloading bytecodes: {} / {} ({:.1}%) | Rate: {} | ETA: {}",
            TOTAL_PHASES,
            format_count(downloaded),
            format_count(total),
            pct,
            format_rate(rate),
            eta_str
        );

        self.mark_logged(&self.bytecodes).await;
    }

    /// Check if we should log (respecting minimum interval)
    async fn should_log(&self, progress: &PhaseProgress) -> bool {
        let last_log = *progress.last_log_time.lock().await;
        match last_log {
            None => true,
            Some(t) => t.elapsed() >= self.log_interval,
        }
    }

    /// Mark that we just logged
    async fn mark_logged(&self, progress: &PhaseProgress) {
        *progress.last_log_time.lock().await = Some(Instant::now());
    }
}

/// Global progress tracker instance
pub static SNAP_PROGRESS: std::sync::LazyLock<Arc<SnapSyncProgress>> =
    std::sync::LazyLock::new(|| Arc::new(SnapSyncProgress::new()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(500), "500");
        assert_eq!(format_count(1500), "1.5K");
        assert_eq!(format_count(1_500_000), "1.50M");
        assert_eq!(format_count(1_500_000_000), "1.50B");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3700)), "1h 1m");
    }

    #[test]
    fn test_format_rate() {
        assert_eq!(format_rate(500.0), "500/s");
        assert_eq!(format_rate(1500.0), "1.5K/s");
        assert_eq!(format_rate(1_500_000.0), "1.5M/s");
    }

    #[test]
    fn test_calculate_eta() {
        let eta = calculate_eta(50, 100, Duration::from_secs(10));
        assert!(eta.is_some());
        // 50 items in 10 seconds = 5/s, 50 remaining = 10 seconds ETA
        assert_eq!(eta.unwrap().as_secs(), 10);
    }
}
