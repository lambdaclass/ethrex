use serde::{Deserialize, Serialize};

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
