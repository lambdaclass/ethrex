//! `compare`: summarize and diff two `run` metrics logs (`--out-log` files).
//!
//! Parses the stable `key=value` metrics lines `run.rs` appends (one per
//! measured run; see its module doc for the exact format), computes
//! mean/median/stddev/coefficient-of-variation per metric for each log, and
//! prints a delta table between the two. Intended for an A/B comparison of two
//! `state-bench` builds (e.g. built from branch A vs branch B) run against
//! the same fixture + workload.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Coefficient-of-variation threshold (percent) above which a metric's spread is
/// flagged as "high variance" rather than treated as a tight, reproducible number.
const HIGH_VARIANCE_COV_PCT: f64 = 5.0;

/// One parsed metrics line (see `run.rs`'s `measure` for the exact producer).
#[derive(Debug, Clone, Copy)]
struct RunMetrics {
    #[allow(dead_code)] // kept for completeness / future use (e.g. per-run diffing)
    run: u64,
    jobs: u64,
    total_seconds: f64,
    loop_seconds: f64,
    commit_seconds: f64,
    ggas: f64,
    block_cache_miss: f64,
    bytes_read: f64,
}

/// Parse one `key=value ...` line into a [`RunMetrics`]. Returns `None` (and
/// thus the line is silently ignored) unless every field this struct needs is
/// present and parses; that keeps non-metric lines (tracing output mixed into
/// the same file, blank lines, etc.) harmless.
fn parse_line(line: &str) -> Option<RunMetrics> {
    let mut fields: HashMap<&str, &str> = HashMap::new();
    for token in line.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            fields.insert(key, value);
        }
    }
    let get_u64 = |k: &str| fields.get(k)?.parse::<u64>().ok();
    let get_f64 = |k: &str| fields.get(k)?.parse::<f64>().ok();

    Some(RunMetrics {
        run: get_u64("run")?,
        jobs: get_u64("jobs")?,
        total_seconds: get_f64("total_seconds")?,
        loop_seconds: get_f64("loop_seconds")?,
        commit_seconds: get_f64("commit_seconds")?,
        ggas: get_f64("ggas")?,
        block_cache_miss: get_f64("block_cache_miss")?,
        bytes_read: get_f64("bytes_read")?,
    })
}

/// Read an out-log and parse every metrics line, ignoring anything else.
fn read_log(path: &Path) -> Result<Vec<RunMetrics>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading out-log {}", path.display()))?;
    Ok(text.lines().filter_map(parse_line).collect())
}

/// Summary statistics for one metric over a set of runs.
#[derive(Debug, Clone, Copy)]
struct Stats {
    n: usize,
    mean: f64,
    median: f64,
    /// `None` when fewer than 2 samples (sample stddev is undefined).
    stddev: Option<f64>,
    /// Coefficient of variation, as a percentage of the mean. `None` alongside
    /// `stddev`.
    cov_pct: Option<f64>,
}

fn compute_stats(xs: &[f64]) -> Stats {
    let n = xs.len();
    let mean = if n == 0 {
        0.0
    } else {
        xs.iter().sum::<f64>() / n as f64
    };
    let median = median(xs);
    let stddev = if n > 1 {
        let variance = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
        Some(variance.sqrt())
    } else {
        None
    };
    let cov_pct = stddev.map(|s| {
        if mean != 0.0 {
            (s / mean.abs()) * 100.0
        } else {
            0.0
        }
    });
    Stats {
        n,
        mean,
        median,
        stddev,
        cov_pct,
    }
}

fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// A row of the printed delta table: one metric, both branches' stats, and the
/// derived %change + significance note.
struct Row {
    name: &'static str,
    a: Stats,
    b: Stats,
    note: String,
}

fn percent_change(a_mean: f64, b_mean: f64) -> Option<f64> {
    if a_mean == 0.0 {
        None
    } else {
        Some((b_mean - a_mean) / a_mean * 100.0)
    }
}

/// Build the significance note for one metric: flags high per-branch variance
/// and whether the delta is smaller than the combined stddev ("within noise").
fn significance_note(a: &Stats, b: &Stats) -> String {
    let mut notes = Vec::new();

    if a.n < 2 || b.n < 2 {
        notes.push("single run, no stddev".to_string());
    } else {
        let combined_stddev = a.stddev.unwrap_or(0.0) + b.stddev.unwrap_or(0.0);
        let delta = (b.mean - a.mean).abs();
        if delta < combined_stddev {
            notes.push("within noise".to_string());
        }
    }

    if let Some(cov) = a.cov_pct
        && cov > HIGH_VARIANCE_COV_PCT
    {
        notes.push(format!("A high variance (CoV {cov:.1}%)"));
    }
    if let Some(cov) = b.cov_pct
        && cov > HIGH_VARIANCE_COV_PCT
    {
        notes.push(format!("B high variance (CoV {cov:.1}%)"));
    }

    if notes.is_empty() {
        "-".to_string()
    } else {
        notes.join("; ")
    }
}

fn fmt_mean_std(s: &Stats) -> String {
    match s.stddev {
        Some(std) => format!("{:.4} \u{00b1} {:.4}", s.mean, std),
        None => format!("{:.4} \u{00b1} n/a", s.mean),
    }
}

fn fmt_pct_change(pct: Option<f64>) -> String {
    match pct {
        Some(p) => format!("{p:+.2}%"),
        None => "n/a".to_string(),
    }
}

/// The `compare` subcommand entrypoint: `state-bench compare <log_a> <log_b>`.
pub fn run(log_a: &Path, log_b: &Path) -> Result<()> {
    let runs_a = read_log(log_a)?;
    let runs_b = read_log(log_b)?;

    if runs_a.is_empty() {
        bail!(
            "log A ({}) has no parseable metrics lines; nothing to compare",
            log_a.display()
        );
    }
    if runs_b.is_empty() {
        bail!(
            "log B ({}) has no parseable metrics lines; nothing to compare",
            log_b.display()
        );
    }

    println!("=== state-bench compare ===");
    println!(
        "A: {} ({} run{})",
        log_a.display(),
        runs_a.len(),
        if runs_a.len() == 1 { "" } else { "s" }
    );
    println!(
        "B: {} ({} run{})",
        log_b.display(),
        runs_b.len(),
        if runs_b.len() == 1 { "" } else { "s" }
    );
    println!();

    // Jobs confound check: the whole comparison is meaningless if A and B were
    // run with different merkleization thread pool sizes.
    let jobs_a = runs_a[0].jobs;
    let jobs_b = runs_b[0].jobs;
    if jobs_a != jobs_b {
        println!(
            "WARNING: jobs mismatch — A ran with jobs={jobs_a}, B ran with jobs={jobs_b}. \
             This comparison is CONFOUNDED: differences below may be caused by parallelism, \
             not by the code under test. Re-run both branches with the same --jobs."
        );
        println!();
    }

    // Compare on the smaller run count if they differ, and say so.
    let n = runs_a.len().min(runs_b.len());
    if runs_a.len() != runs_b.len() {
        println!(
            "NOTE: run count mismatch (A={}, B={}); comparing the first {n} run(s) of each.",
            runs_a.len(),
            runs_b.len()
        );
        println!();
    }
    let runs_a = &runs_a[..n];
    let runs_b = &runs_b[..n];

    type MetricExtractor = fn(&RunMetrics) -> f64;
    let metrics: [(&str, MetricExtractor); 6] = [
        ("total_seconds", |m| m.total_seconds),
        ("loop_seconds", |m| m.loop_seconds),
        ("commit_seconds", |m| m.commit_seconds),
        ("ggas", |m| m.ggas),
        ("block_cache_miss", |m| m.block_cache_miss),
        ("bytes_read", |m| m.bytes_read),
    ];

    let rows: Vec<Row> = metrics
        .iter()
        .map(|(name, extract)| {
            let a_vals: Vec<f64> = runs_a.iter().map(extract).collect();
            let b_vals: Vec<f64> = runs_b.iter().map(extract).collect();
            let a = compute_stats(&a_vals);
            let b = compute_stats(&b_vals);
            let note = significance_note(&a, &b);
            Row { name, a, b, note }
        })
        .collect();

    print_table(&rows);
    Ok(())
}

fn print_table(rows: &[Row]) {
    const COL_METRIC: usize = 17;
    const COL_STATS: usize = 24;
    const COL_PCT: usize = 10;

    println!(
        "{:<COL_METRIC$} | {:<COL_STATS$} | {:<COL_STATS$} | {:<COL_PCT$} | note",
        "metric", "A (mean \u{00b1} std)", "B (mean \u{00b1} std)", "% change"
    );
    println!(
        "{}-+-{}-+-{}-+-{}-+-{}",
        "-".repeat(COL_METRIC),
        "-".repeat(COL_STATS),
        "-".repeat(COL_STATS),
        "-".repeat(COL_PCT),
        "-".repeat(20)
    );
    for row in rows {
        let pct = percent_change(row.a.mean, row.b.mean);
        println!(
            "{:<COL_METRIC$} | {:<COL_STATS$} | {:<COL_STATS$} | {:<COL_PCT$} | {}",
            row.name,
            fmt_mean_std(&row.a),
            fmt_mean_std(&row.b),
            fmt_pct_change(pct),
            row.note,
        );
    }
    println!();
    println!(
        "medians — A: {}",
        rows.iter()
            .map(|r| format!("{}={:.4}", r.name, r.a.median))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "medians — B: {}",
        rows.iter()
            .map(|r| format!("{}={:.4}", r.name, r.b.median))
            .collect::<Vec<_>>()
            .join(", ")
    );
}
