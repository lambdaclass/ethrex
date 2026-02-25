use serde::{Deserialize, Serialize};

use crate::stats::BenchStats;

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchSuite {
    pub timestamp: String,
    pub commit: String,
    pub results: Vec<BenchResult>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchResult {
    pub scenario: String,
    pub total_duration_ns: u128,
    pub runs: u64,
    pub opcode_timings: Vec<OpcodeEntry>,
    /// Statistical summary of per-run durations (None if < 2 samples).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<BenchStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpcodeEntry {
    pub opcode: String,
    pub avg_ns: u128,
    pub total_ns: u128,
    pub count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegressionReport {
    pub status: RegressionStatus,
    pub thresholds: Thresholds,
    pub regressions: Vec<Regression>,
    pub improvements: Vec<Regression>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegressionStatus {
    Stable,
    Warning,
    Regression,
}

impl std::fmt::Display for RegressionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stable => write!(f, "Stable"),
            Self::Warning => write!(f, "Warning"),
            Self::Regression => write!(f, "Regression"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Regression {
    pub scenario: String,
    pub opcode: String,
    pub baseline_avg_ns: u128,
    pub current_avg_ns: u128,
    pub change_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    pub warning_percent: f64,
    pub regression_percent: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            warning_percent: 20.0,
            regression_percent: 50.0,
        }
    }
}

/// Result of a JIT vs interpreter benchmark comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JitBenchResult {
    /// Name of the benchmark scenario.
    pub scenario: String,
    /// Interpreter execution time in nanoseconds.
    pub interpreter_ns: u128,
    /// JIT execution time in nanoseconds (None if revmc-backend not available).
    pub jit_ns: Option<u128>,
    /// Speedup ratio (interpreter_ns / jit_ns). None if JIT not available.
    pub speedup: Option<f64>,
    /// Number of iterations.
    pub runs: u64,
    /// Interpreter per-run statistics (None if < 2 samples).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interp_stats: Option<BenchStats>,
    /// JIT per-run statistics (None if < 2 samples or JIT unavailable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jit_stats: Option<BenchStats>,
}

/// A full JIT benchmark suite with metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct JitBenchSuite {
    /// Unix timestamp of the benchmark run.
    pub timestamp: String,
    /// Git commit hash.
    pub commit: String,
    /// Results for each scenario.
    pub results: Vec<JitBenchResult>,
}

/// A single scenario's JIT speedup regression entry.
#[derive(Debug, Serialize, Deserialize)]
pub struct JitSpeedupDelta {
    pub scenario: String,
    pub baseline_speedup: f64,
    pub current_speedup: f64,
    /// Negative = regression (speedup dropped).
    pub change_percent: f64,
}

/// Report comparing JIT speedup ratios between baseline and current.
#[derive(Debug, Serialize, Deserialize)]
pub struct JitRegressionReport {
    pub status: RegressionStatus,
    pub threshold_percent: f64,
    pub regressions: Vec<JitSpeedupDelta>,
    pub improvements: Vec<JitSpeedupDelta>,
}
