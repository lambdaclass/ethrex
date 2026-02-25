use crate::types::{BenchSuite, JitBenchSuite, JitRegressionReport, RegressionReport};

pub fn to_json(suite: &BenchSuite) -> String {
    serde_json::to_string_pretty(suite).expect("Failed to serialize BenchSuite")
}

pub fn from_json(json: &str) -> BenchSuite {
    serde_json::from_str(json).expect("Failed to deserialize BenchSuite")
}

pub fn regression_to_json(report: &RegressionReport) -> String {
    serde_json::to_string_pretty(report).expect("Failed to serialize RegressionReport")
}

pub fn regression_from_json(json: &str) -> RegressionReport {
    serde_json::from_str(json).expect("Failed to deserialize RegressionReport")
}

pub fn to_markdown(report: &RegressionReport) -> String {
    let mut md = String::new();

    md.push_str(&format!(
        "## Tokamak Benchmark Results: **{}**\n\n",
        report.status
    ));

    if report.regressions.is_empty() && report.improvements.is_empty() {
        md.push_str("No significant changes detected.\n");
        return md;
    }

    if !report.regressions.is_empty() {
        md.push_str("### Regressions\n\n");
        md.push_str("| Scenario | Opcode | Baseline (ns) | Current (ns) | Change | Status |\n");
        md.push_str("|----------|--------|---------------|--------------|--------|--------|\n");
        for r in &report.regressions {
            let status = if r.change_percent >= report.thresholds.regression_percent {
                "REGRESSION"
            } else {
                "WARNING"
            };
            md.push_str(&format!(
                "| {} | {} | {} | {} | {:+.1}% | {} |\n",
                r.scenario, r.opcode, r.baseline_avg_ns, r.current_avg_ns, r.change_percent, status
            ));
        }
        md.push('\n');
    }

    if !report.improvements.is_empty() {
        md.push_str("### Improvements\n\n");
        md.push_str("| Scenario | Opcode | Baseline (ns) | Current (ns) | Change |\n");
        md.push_str("|----------|--------|---------------|--------------|--------|\n");
        for r in &report.improvements {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {:+.1}% |\n",
                r.scenario, r.opcode, r.baseline_avg_ns, r.current_avg_ns, r.change_percent
            ));
        }
        md.push('\n');
    }

    md
}

pub fn jit_suite_to_json(suite: &JitBenchSuite) -> String {
    serde_json::to_string_pretty(suite).expect("Failed to serialize JitBenchSuite")
}

pub fn jit_suite_from_json(json: &str) -> JitBenchSuite {
    serde_json::from_str(json).expect("Failed to deserialize JitBenchSuite")
}

#[expect(clippy::as_conversions, reason = "ns-to-ms conversion for display")]
pub fn jit_to_markdown(suite: &JitBenchSuite) -> String {
    let mut md = String::new();

    md.push_str("## JIT vs Interpreter Benchmark\n\n");
    md.push_str(&format!("Commit: `{}`\n\n", suite.commit));
    md.push_str("| Scenario | Interpreter (ms) | JIT (ms) | Speedup | Interp Stddev (ms) | JIT Stddev (ms) |\n");
    md.push_str("|----------|------------------|----------|---------|--------------------|-----------------|\n");

    for result in &suite.results {
        let interp_ms = result.interpreter_ns as f64 / 1_000_000.0;
        let jit_ms = result
            .jit_ns
            .map(|ns| ns as f64 / 1_000_000.0)
            .unwrap_or(0.0);
        let speedup = result
            .speedup
            .map(|s| format!("{s:.2}x"))
            .unwrap_or_else(|| "N/A".to_string());

        let interp_stddev = result
            .interp_stats
            .as_ref()
            .map(|s| format!("{:.3}", s.stddev_ns / 1_000_000.0))
            .unwrap_or_else(|| "N/A".to_string());
        let jit_stddev = result
            .jit_stats
            .as_ref()
            .map(|s| format!("{:.3}", s.stddev_ns / 1_000_000.0))
            .unwrap_or_else(|| "N/A".to_string());

        md.push_str(&format!(
            "| {} | {interp_ms:.3} | {jit_ms:.3} | {speedup} | {interp_stddev} | {jit_stddev} |\n",
            result.scenario,
        ));
    }

    md.push('\n');
    md
}

pub fn jit_regression_to_json(report: &JitRegressionReport) -> String {
    serde_json::to_string_pretty(report).expect("Failed to serialize JitRegressionReport")
}

pub fn jit_regression_from_json(json: &str) -> JitRegressionReport {
    serde_json::from_str(json).expect("Failed to deserialize JitRegressionReport")
}

pub fn jit_regression_to_markdown(report: &JitRegressionReport) -> String {
    let mut md = String::new();

    md.push_str(&format!(
        "## JIT Speedup Regression: **{}**\n\n",
        report.status
    ));
    md.push_str(&format!(
        "Threshold: {:.0}% speedup drop\n\n",
        report.threshold_percent
    ));

    if report.regressions.is_empty() && report.improvements.is_empty() {
        md.push_str("No significant JIT speedup changes detected.\n");
        return md;
    }

    if !report.regressions.is_empty() {
        md.push_str("### Regressions\n\n");
        md.push_str("| Scenario | Baseline Speedup | Current Speedup | Change |\n");
        md.push_str("|----------|-----------------|-----------------|--------|\n");
        for r in &report.regressions {
            md.push_str(&format!(
                "| {} | {:.2}x | {:.2}x | {:+.1}% |\n",
                r.scenario, r.baseline_speedup, r.current_speedup, r.change_percent
            ));
        }
        md.push('\n');
    }

    if !report.improvements.is_empty() {
        md.push_str("### Improvements\n\n");
        md.push_str("| Scenario | Baseline Speedup | Current Speedup | Change |\n");
        md.push_str("|----------|-----------------|-----------------|--------|\n");
        for r in &report.improvements {
            md.push_str(&format!(
                "| {} | {:.2}x | {:.2}x | {:+.1}% |\n",
                r.scenario, r.baseline_speedup, r.current_speedup, r.change_percent
            ));
        }
        md.push('\n');
    }

    md
}

/// Generate a suite-level statistics markdown section.
#[expect(clippy::as_conversions, reason = "ns-to-ms conversion for display")]
pub fn suite_stats_to_markdown(suite: &BenchSuite) -> String {
    let mut md = String::new();

    md.push_str("## Scenario Statistics\n\n");
    md.push_str(
        "| Scenario | Mean (ms) | Stddev (ms) | 95% CI (ms) | Min (ms) | Max (ms) | Runs |\n",
    );
    md.push_str(
        "|----------|-----------|-------------|-------------|----------|----------|------|\n",
    );

    for result in &suite.results {
        if let Some(ref s) = result.stats {
            md.push_str(&format!(
                "| {} | {:.3} | {:.3} | [{:.3}, {:.3}] | {:.3} | {:.3} | {} |\n",
                result.scenario,
                s.mean_ns / 1_000_000.0,
                s.stddev_ns / 1_000_000.0,
                s.ci_lower_ns / 1_000_000.0,
                s.ci_upper_ns / 1_000_000.0,
                s.min_ns as f64 / 1_000_000.0,
                s.max_ns as f64 / 1_000_000.0,
                s.samples,
            ));
        } else {
            md.push_str(&format!(
                "| {} | {:.3} | N/A | N/A | N/A | N/A | {} |\n",
                result.scenario,
                result.total_duration_ns as f64 / 1_000_000.0 / result.runs as f64,
                result.runs,
            ));
        }
    }

    md.push('\n');
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BenchResult, JitBenchResult, JitRegressionReport, JitSpeedupDelta, OpcodeEntry,
        RegressionStatus, Thresholds,
    };

    #[test]
    fn test_json_roundtrip() {
        let suite = BenchSuite {
            timestamp: "1234567890".to_string(),
            commit: "abc123".to_string(),
            results: vec![BenchResult {
                scenario: "Fibonacci".to_string(),
                total_duration_ns: 1_000_000,
                runs: 10,
                opcode_timings: vec![OpcodeEntry {
                    opcode: "ADD".to_string(),
                    avg_ns: 100,
                    total_ns: 1000,
                    count: 10,
                }],
                stats: None,
            }],
        };

        let json = to_json(&suite);
        let parsed = from_json(&json);
        assert_eq!(parsed.commit, "abc123");
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].scenario, "Fibonacci");
    }

    #[test]
    fn test_markdown_output() {
        let report = RegressionReport {
            status: RegressionStatus::Stable,
            thresholds: Thresholds::default(),
            regressions: vec![],
            improvements: vec![],
        };
        let md = to_markdown(&report);
        assert!(md.contains("Stable"));
        assert!(md.contains("No significant changes"));
    }

    #[test]
    fn test_regression_json_roundtrip() {
        let report = RegressionReport {
            status: RegressionStatus::Warning,
            thresholds: Thresholds::default(),
            regressions: vec![],
            improvements: vec![],
        };
        let json = regression_to_json(&report);
        let parsed = regression_from_json(&json);
        assert_eq!(parsed.status, RegressionStatus::Warning);
    }

    #[test]
    fn test_jit_suite_json_roundtrip() {
        let suite = JitBenchSuite {
            timestamp: "1234567890".to_string(),
            commit: "abc123".to_string(),
            results: vec![JitBenchResult {
                scenario: "Fibonacci".to_string(),
                interpreter_ns: 10_000_000,
                jit_ns: Some(2_000_000),
                speedup: Some(5.0),
                runs: 10,
                interp_stats: None,
                jit_stats: None,
            }],
        };
        let json = jit_suite_to_json(&suite);
        let parsed = jit_suite_from_json(&json);
        assert_eq!(parsed.commit, "abc123");
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].speedup, Some(5.0));
    }

    #[test]
    fn test_jit_markdown_output() {
        let suite = JitBenchSuite {
            timestamp: "0".to_string(),
            commit: "test123".to_string(),
            results: vec![
                JitBenchResult {
                    scenario: "Fibonacci".to_string(),
                    interpreter_ns: 12_340_000,
                    jit_ns: Some(2_100_000),
                    speedup: Some(5.876),
                    runs: 10,
                    interp_stats: None,
                    jit_stats: None,
                },
                JitBenchResult {
                    scenario: "ERC20Transfer".to_string(),
                    interpreter_ns: 8_560_000,
                    jit_ns: None,
                    speedup: None,
                    runs: 10,
                    interp_stats: None,
                    jit_stats: None,
                },
            ],
        };
        let md = jit_to_markdown(&suite);
        assert!(md.contains("JIT vs Interpreter Benchmark"));
        assert!(md.contains("Fibonacci"));
        assert!(md.contains("ERC20Transfer"));
        assert!(md.contains("test123"));
        assert!(md.contains("N/A"));
    }

    #[test]
    fn test_jit_regression_json_roundtrip() {
        let report = JitRegressionReport {
            status: RegressionStatus::Regression,
            threshold_percent: 20.0,
            regressions: vec![JitSpeedupDelta {
                scenario: "Fibonacci".to_string(),
                baseline_speedup: 2.5,
                current_speedup: 1.8,
                change_percent: -28.0,
            }],
            improvements: vec![],
        };
        let json = jit_regression_to_json(&report);
        let parsed = jit_regression_from_json(&json);
        assert_eq!(parsed.status, RegressionStatus::Regression);
        assert_eq!(parsed.regressions.len(), 1);
    }

    #[test]
    fn test_jit_regression_markdown_stable() {
        let report = JitRegressionReport {
            status: RegressionStatus::Stable,
            threshold_percent: 20.0,
            regressions: vec![],
            improvements: vec![],
        };
        let md = jit_regression_to_markdown(&report);
        assert!(md.contains("Stable"));
        assert!(md.contains("No significant"));
    }

    #[test]
    fn test_jit_regression_markdown_with_entries() {
        let report = JitRegressionReport {
            status: RegressionStatus::Regression,
            threshold_percent: 20.0,
            regressions: vec![JitSpeedupDelta {
                scenario: "BubbleSort".to_string(),
                baseline_speedup: 2.24,
                current_speedup: 1.50,
                change_percent: -33.0,
            }],
            improvements: vec![JitSpeedupDelta {
                scenario: "Fibonacci".to_string(),
                baseline_speedup: 2.5,
                current_speedup: 3.2,
                change_percent: 28.0,
            }],
        };
        let md = jit_regression_to_markdown(&report);
        assert!(md.contains("Regression"));
        assert!(md.contains("BubbleSort"));
        assert!(md.contains("2.24x"));
        assert!(md.contains("Fibonacci"));
        assert!(md.contains("Improvements"));
    }

    #[test]
    fn test_suite_stats_markdown() {
        use crate::stats::BenchStats;
        let suite = BenchSuite {
            timestamp: "0".to_string(),
            commit: "test".to_string(),
            results: vec![BenchResult {
                scenario: "Fibonacci".to_string(),
                total_duration_ns: 35_500_000,
                runs: 10,
                opcode_timings: vec![],
                stats: Some(BenchStats {
                    mean_ns: 3_550_000.0,
                    stddev_ns: 120_000.0,
                    ci_lower_ns: 3_475_000.0,
                    ci_upper_ns: 3_625_000.0,
                    min_ns: 3_410_000,
                    max_ns: 3_780_000,
                    samples: 10,
                }),
            }],
        };
        let md = suite_stats_to_markdown(&suite);
        assert!(md.contains("Fibonacci"));
        assert!(md.contains("Stddev"));
        assert!(md.contains("95% CI"));
    }
}
