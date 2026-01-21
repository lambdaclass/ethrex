//! Database metrics for observability.
//!
//! Tracks statistics about database operations for monitoring and debugging.

use std::sync::atomic::{AtomicU64, Ordering};

/// Database metrics container.
///
/// All counters are atomic for thread-safe access.
#[derive(Debug, Default)]
pub struct DbMetrics {
    /// Number of pages allocated.
    pub pages_allocated: AtomicU64,
    /// Number of pages reused from abandoned pool.
    pub pages_reused: AtomicU64,
    /// Number of pages abandoned.
    pub pages_abandoned: AtomicU64,
    /// Number of page reads.
    pub page_reads: AtomicU64,
    /// Number of page writes.
    pub page_writes: AtomicU64,
    /// Number of COW operations (get_writable_copy).
    pub cow_operations: AtomicU64,
    /// Number of batches committed.
    pub batches_committed: AtomicU64,
    /// Number of batches aborted.
    pub batches_aborted: AtomicU64,
    /// Number of snapshots created.
    pub snapshots_created: AtomicU64,
    /// Number of snapshot restores.
    pub snapshots_restored: AtomicU64,
    /// Total bytes written to pages.
    pub bytes_written: AtomicU64,
    /// Total bytes read from pages.
    pub bytes_read: AtomicU64,
}

impl DbMetrics {
    /// Creates a new metrics container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increments the page allocation counter.
    pub fn inc_pages_allocated(&self) {
        self.pages_allocated.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the page reuse counter.
    pub fn inc_pages_reused(&self) {
        self.pages_reused.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the page abandoned counter.
    pub fn inc_pages_abandoned(&self) {
        self.pages_abandoned.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds to pages abandoned counter.
    pub fn add_pages_abandoned(&self, count: u64) {
        self.pages_abandoned.fetch_add(count, Ordering::Relaxed);
    }

    /// Increments the page read counter.
    pub fn inc_page_reads(&self) {
        self.page_reads.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the page write counter.
    pub fn inc_page_writes(&self) {
        self.page_writes.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds to page writes counter.
    pub fn add_page_writes(&self, count: u64) {
        self.page_writes.fetch_add(count, Ordering::Relaxed);
    }

    /// Increments the COW operation counter.
    pub fn inc_cow_operations(&self) {
        self.cow_operations.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the batch committed counter.
    pub fn inc_batches_committed(&self) {
        self.batches_committed.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the batch aborted counter.
    pub fn inc_batches_aborted(&self) {
        self.batches_aborted.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the snapshot created counter.
    pub fn inc_snapshots_created(&self) {
        self.snapshots_created.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the snapshot restored counter.
    pub fn inc_snapshots_restored(&self) {
        self.snapshots_restored.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds bytes to the written counter.
    pub fn add_bytes_written(&self, bytes: u64) {
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Adds bytes to the read counter.
    pub fn add_bytes_read(&self, bytes: u64) {
        self.bytes_read.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Returns a snapshot of all metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            pages_allocated: self.pages_allocated.load(Ordering::Relaxed),
            pages_reused: self.pages_reused.load(Ordering::Relaxed),
            pages_abandoned: self.pages_abandoned.load(Ordering::Relaxed),
            page_reads: self.page_reads.load(Ordering::Relaxed),
            page_writes: self.page_writes.load(Ordering::Relaxed),
            cow_operations: self.cow_operations.load(Ordering::Relaxed),
            batches_committed: self.batches_committed.load(Ordering::Relaxed),
            batches_aborted: self.batches_aborted.load(Ordering::Relaxed),
            snapshots_created: self.snapshots_created.load(Ordering::Relaxed),
            snapshots_restored: self.snapshots_restored.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
        }
    }

    /// Resets all metrics to zero.
    pub fn reset(&self) {
        self.pages_allocated.store(0, Ordering::Relaxed);
        self.pages_reused.store(0, Ordering::Relaxed);
        self.pages_abandoned.store(0, Ordering::Relaxed);
        self.page_reads.store(0, Ordering::Relaxed);
        self.page_writes.store(0, Ordering::Relaxed);
        self.cow_operations.store(0, Ordering::Relaxed);
        self.batches_committed.store(0, Ordering::Relaxed);
        self.batches_aborted.store(0, Ordering::Relaxed);
        self.snapshots_created.store(0, Ordering::Relaxed);
        self.snapshots_restored.store(0, Ordering::Relaxed);
        self.bytes_written.store(0, Ordering::Relaxed);
        self.bytes_read.store(0, Ordering::Relaxed);
    }
}

/// A point-in-time snapshot of metrics values.
#[derive(Debug, Clone, Copy)]
pub struct MetricsSnapshot {
    pub pages_allocated: u64,
    pub pages_reused: u64,
    pub pages_abandoned: u64,
    pub page_reads: u64,
    pub page_writes: u64,
    pub cow_operations: u64,
    pub batches_committed: u64,
    pub batches_aborted: u64,
    pub snapshots_created: u64,
    pub snapshots_restored: u64,
    pub bytes_written: u64,
    pub bytes_read: u64,
}

impl MetricsSnapshot {
    /// Calculates the difference between two snapshots.
    pub fn diff(&self, other: &MetricsSnapshot) -> MetricsSnapshot {
        MetricsSnapshot {
            pages_allocated: self.pages_allocated.saturating_sub(other.pages_allocated),
            pages_reused: self.pages_reused.saturating_sub(other.pages_reused),
            pages_abandoned: self.pages_abandoned.saturating_sub(other.pages_abandoned),
            page_reads: self.page_reads.saturating_sub(other.page_reads),
            page_writes: self.page_writes.saturating_sub(other.page_writes),
            cow_operations: self.cow_operations.saturating_sub(other.cow_operations),
            batches_committed: self
                .batches_committed
                .saturating_sub(other.batches_committed),
            batches_aborted: self.batches_aborted.saturating_sub(other.batches_aborted),
            snapshots_created: self
                .snapshots_created
                .saturating_sub(other.snapshots_created),
            snapshots_restored: self
                .snapshots_restored
                .saturating_sub(other.snapshots_restored),
            bytes_written: self.bytes_written.saturating_sub(other.bytes_written),
            bytes_read: self.bytes_read.saturating_sub(other.bytes_read),
        }
    }

    /// Returns the page reuse rate (0.0 - 1.0).
    pub fn reuse_rate(&self) -> f64 {
        let total = self.pages_allocated + self.pages_reused;
        if total == 0 {
            0.0
        } else {
            self.pages_reused as f64 / total as f64
        }
    }

    /// Returns the average bytes per page write.
    pub fn avg_bytes_per_write(&self) -> f64 {
        if self.page_writes == 0 {
            0.0
        } else {
            self.bytes_written as f64 / self.page_writes as f64
        }
    }
}

impl std::fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Database Metrics:")?;
        writeln!(f, "  Pages allocated:  {}", self.pages_allocated)?;
        writeln!(
            f,
            "  Pages reused:     {} ({:.1}%)",
            self.pages_reused,
            self.reuse_rate() * 100.0
        )?;
        writeln!(f, "  Pages abandoned:  {}", self.pages_abandoned)?;
        writeln!(f, "  Page reads:       {}", self.page_reads)?;
        writeln!(f, "  Page writes:      {}", self.page_writes)?;
        writeln!(f, "  COW operations:   {}", self.cow_operations)?;
        writeln!(f, "  Batches committed: {}", self.batches_committed)?;
        writeln!(f, "  Batches aborted:  {}", self.batches_aborted)?;
        writeln!(f, "  Snapshots created: {}", self.snapshots_created)?;
        writeln!(f, "  Snapshots restored: {}", self.snapshots_restored)?;
        writeln!(
            f,
            "  Bytes written:    {} ({:.2} KB)",
            self.bytes_written,
            self.bytes_written as f64 / 1024.0
        )?;
        writeln!(
            f,
            "  Bytes read:       {} ({:.2} KB)",
            self.bytes_read,
            self.bytes_read as f64 / 1024.0
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_increment() {
        let metrics = DbMetrics::new();

        metrics.inc_pages_allocated();
        metrics.inc_pages_allocated();
        metrics.inc_pages_reused();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.pages_allocated, 2);
        assert_eq!(snapshot.pages_reused, 1);
    }

    #[test]
    fn test_metrics_snapshot_diff() {
        let metrics = DbMetrics::new();

        metrics.inc_page_reads();
        metrics.inc_page_reads();
        let snap1 = metrics.snapshot();

        metrics.inc_page_reads();
        metrics.inc_page_reads();
        metrics.inc_page_reads();
        let snap2 = metrics.snapshot();

        let diff = snap2.diff(&snap1);
        assert_eq!(diff.page_reads, 3);
    }

    #[test]
    fn test_reuse_rate() {
        let snapshot = MetricsSnapshot {
            pages_allocated: 80,
            pages_reused: 20,
            pages_abandoned: 0,
            page_reads: 0,
            page_writes: 0,
            cow_operations: 0,
            batches_committed: 0,
            batches_aborted: 0,
            snapshots_created: 0,
            snapshots_restored: 0,
            bytes_written: 0,
            bytes_read: 0,
        };

        assert!((snapshot.reuse_rate() - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_metrics_reset() {
        let metrics = DbMetrics::new();

        metrics.inc_pages_allocated();
        metrics.inc_batches_committed();

        metrics.reset();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.pages_allocated, 0);
        assert_eq!(snapshot.batches_committed, 0);
    }
}
