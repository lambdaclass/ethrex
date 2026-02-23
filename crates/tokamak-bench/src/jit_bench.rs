//! JIT compilation benchmarks.
//!
//! Compares Fibonacci execution time between the LEVM interpreter and
//! JIT-compiled code (when `revmc-backend` feature is enabled on tokamak-jit).
//!
//! This module only provides the benchmark data structures and interpreter
//! baseline measurement. The actual JIT comparison requires LLVM and is
//! gated behind tokamak-jit's `revmc-backend` feature.

use std::time::Duration;

/// Result of a JIT vs interpreter benchmark comparison.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JitBenchResult {
    /// Name of the benchmark scenario.
    pub scenario: String,
    /// Interpreter execution time.
    pub interpreter_ns: u128,
    /// JIT execution time (None if revmc-backend not available).
    pub jit_ns: Option<u128>,
    /// Speedup ratio (interpreter_ns / jit_ns). None if JIT not available.
    pub speedup: Option<f64>,
    /// Number of iterations.
    pub runs: u64,
}

/// Measure interpreter execution time for a given scenario.
///
/// This serves as the baseline for JIT comparison benchmarks.
/// The actual bytecode execution uses the same setup as `runner::run_scenario`.
pub fn measure_interpreter_baseline(
    scenario_name: &str,
    bytecode_hex: &str,
    iterations: u64,
    runs: u64,
) -> Duration {
    use crate::runner::run_scenario;

    let result = run_scenario(scenario_name, bytecode_hex, runs, iterations);
    Duration::from_nanos(u64::try_from(result.total_duration_ns).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jit_bench_result_serialization() {
        let result = JitBenchResult {
            scenario: "Fibonacci".to_string(),
            interpreter_ns: 1_000_000,
            jit_ns: Some(200_000),
            speedup: Some(5.0),
            runs: 100,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: JitBenchResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.scenario, "Fibonacci");
        assert_eq!(deserialized.speedup, Some(5.0));
    }

    #[test]
    fn test_jit_bench_result_no_jit() {
        let result = JitBenchResult {
            scenario: "Test".to_string(),
            interpreter_ns: 500_000,
            jit_ns: None,
            speedup: None,
            runs: 10,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("\"jit_ns\":null"));
    }
}
