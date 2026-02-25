use crate::types::{
    BenchSuite, JitBenchSuite, JitRegressionReport, JitSpeedupDelta, Regression, RegressionReport,
    RegressionStatus, Thresholds,
};

/// Compare two benchmark suites and detect regressions.
pub fn compare(
    baseline: &BenchSuite,
    current: &BenchSuite,
    thresholds: &Thresholds,
) -> RegressionReport {
    let mut regressions = Vec::new();
    let mut improvements = Vec::new();
    let mut worst_status = RegressionStatus::Stable;

    for current_result in &current.results {
        let baseline_result = match baseline
            .results
            .iter()
            .find(|b| b.scenario == current_result.scenario)
        {
            Some(b) => b,
            None => continue,
        };

        // Compare top opcodes by total time
        for current_op in &current_result.opcode_timings {
            let baseline_op = match baseline_result
                .opcode_timings
                .iter()
                .find(|b| b.opcode == current_op.opcode)
            {
                Some(b) => b,
                None => continue,
            };

            if baseline_op.avg_ns == 0 {
                continue;
            }

            let change_percent = ((current_op.avg_ns as f64 - baseline_op.avg_ns as f64)
                / baseline_op.avg_ns as f64)
                * 100.0;

            let entry = Regression {
                scenario: current_result.scenario.clone(),
                opcode: current_op.opcode.clone(),
                baseline_avg_ns: baseline_op.avg_ns,
                current_avg_ns: current_op.avg_ns,
                change_percent,
            };

            if change_percent >= thresholds.regression_percent {
                worst_status = RegressionStatus::Regression;
                regressions.push(entry);
            } else if change_percent >= thresholds.warning_percent {
                if worst_status != RegressionStatus::Regression {
                    worst_status = RegressionStatus::Warning;
                }
                regressions.push(entry);
            } else if change_percent <= -thresholds.warning_percent {
                improvements.push(entry);
            }
        }
    }

    // Sort regressions by change_percent descending (worst first)
    regressions.sort_by(|a, b| {
        b.change_percent
            .partial_cmp(&a.change_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Sort improvements by change_percent ascending (best first)
    improvements.sort_by(|a, b| {
        a.change_percent
            .partial_cmp(&b.change_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    RegressionReport {
        status: worst_status,
        thresholds: thresholds.clone(),
        regressions,
        improvements,
    }
}

/// Compare two JIT benchmark suites and detect speedup regressions.
///
/// A "regression" means the JIT speedup ratio dropped by more than
/// `threshold_percent` (e.g., 2.5x → 2.0x = -20%).
pub fn compare_jit(
    baseline: &JitBenchSuite,
    current: &JitBenchSuite,
    threshold_percent: f64,
) -> JitRegressionReport {
    let mut regressions = Vec::new();
    let mut improvements = Vec::new();
    let mut worst_status = RegressionStatus::Stable;

    for current_result in &current.results {
        let current_speedup = match current_result.speedup {
            Some(s) => s,
            None => continue,
        };

        let baseline_result = match baseline
            .results
            .iter()
            .find(|b| b.scenario == current_result.scenario)
        {
            Some(b) => b,
            None => continue,
        };

        let baseline_speedup = match baseline_result.speedup {
            Some(s) => s,
            None => continue,
        };

        if baseline_speedup <= 0.0 {
            continue;
        }

        // Positive = improvement, negative = regression
        let change_percent = ((current_speedup - baseline_speedup) / baseline_speedup) * 100.0;

        let entry = JitSpeedupDelta {
            scenario: current_result.scenario.clone(),
            baseline_speedup,
            current_speedup,
            change_percent,
        };

        if change_percent <= -threshold_percent {
            worst_status = RegressionStatus::Regression;
            regressions.push(entry);
        } else if change_percent >= threshold_percent {
            improvements.push(entry);
        }
    }

    // Sort regressions by change_percent ascending (worst drop first)
    regressions.sort_by(|a, b| {
        a.change_percent
            .partial_cmp(&b.change_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Sort improvements by change_percent descending (best first)
    improvements.sort_by(|a, b| {
        b.change_percent
            .partial_cmp(&a.change_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    JitRegressionReport {
        status: worst_status,
        threshold_percent,
        regressions,
        improvements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BenchResult, JitBenchResult, OpcodeEntry};

    fn make_suite(scenario: &str, opcode: &str, avg_ns: u128) -> BenchSuite {
        BenchSuite {
            timestamp: "0".to_string(),
            commit: "test".to_string(),
            results: vec![BenchResult {
                scenario: scenario.to_string(),
                total_duration_ns: avg_ns * 100,
                runs: 10,
                opcode_timings: vec![OpcodeEntry {
                    opcode: opcode.to_string(),
                    avg_ns,
                    total_ns: avg_ns * 100,
                    count: 100,
                }],
                stats: None,
            }],
        }
    }

    #[test]
    fn test_stable_when_same_data() {
        let suite = make_suite("Fibonacci", "ADD", 100);
        let report = compare(&suite, &suite, &Thresholds::default());
        assert_eq!(report.status, RegressionStatus::Stable);
        assert!(report.regressions.is_empty());
        assert!(report.improvements.is_empty());
    }

    #[test]
    fn test_detects_regression() {
        let baseline = make_suite("Fibonacci", "ADD", 100);
        let current = make_suite("Fibonacci", "ADD", 200); // 100% increase
        let report = compare(&baseline, &current, &Thresholds::default());
        assert_eq!(report.status, RegressionStatus::Regression);
        assert_eq!(report.regressions.len(), 1);
        assert!(report.regressions[0].change_percent >= 50.0);
    }

    #[test]
    fn test_detects_warning() {
        let baseline = make_suite("Fibonacci", "ADD", 100);
        let current = make_suite("Fibonacci", "ADD", 130); // 30% increase
        let report = compare(&baseline, &current, &Thresholds::default());
        assert_eq!(report.status, RegressionStatus::Warning);
        assert_eq!(report.regressions.len(), 1);
    }

    #[test]
    fn test_detects_improvement() {
        let baseline = make_suite("Fibonacci", "ADD", 100);
        let current = make_suite("Fibonacci", "ADD", 50); // 50% decrease
        let report = compare(&baseline, &current, &Thresholds::default());
        assert_eq!(report.status, RegressionStatus::Stable);
        assert!(report.regressions.is_empty());
        assert_eq!(report.improvements.len(), 1);
    }

    #[test]
    fn test_missing_scenario_skipped() {
        let baseline = make_suite("Fibonacci", "ADD", 100);
        let current = make_suite("Unknown", "ADD", 200);
        let report = compare(&baseline, &current, &Thresholds::default());
        assert_eq!(report.status, RegressionStatus::Stable);
    }

    #[test]
    fn test_custom_thresholds() {
        let baseline = make_suite("Fibonacci", "ADD", 100);
        let current = make_suite("Fibonacci", "ADD", 115); // 15% increase
        let thresholds = Thresholds {
            warning_percent: 10.0,
            regression_percent: 20.0,
        };
        let report = compare(&baseline, &current, &thresholds);
        assert_eq!(report.status, RegressionStatus::Warning);
    }

    // ─── JIT speedup regression tests ────────────────────────────────────

    fn make_jit_suite(scenarios: &[(&str, f64)]) -> JitBenchSuite {
        JitBenchSuite {
            timestamp: "0".to_string(),
            commit: "test".to_string(),
            results: scenarios
                .iter()
                .map(|(name, speedup)| JitBenchResult {
                    scenario: name.to_string(),
                    interpreter_ns: 10_000_000,
                    jit_ns: Some((10_000_000.0 / speedup) as u128),
                    speedup: Some(*speedup),
                    runs: 10,
                    interp_stats: None,
                    jit_stats: None,
                })
                .collect(),
        }
    }

    #[test]
    fn test_jit_stable_when_same() {
        let suite = make_jit_suite(&[("Fibonacci", 2.5)]);
        let report = compare_jit(&suite, &suite, 20.0);
        assert_eq!(report.status, RegressionStatus::Stable);
        assert!(report.regressions.is_empty());
        assert!(report.improvements.is_empty());
    }

    #[test]
    fn test_jit_detects_regression() {
        let baseline = make_jit_suite(&[("Fibonacci", 2.5)]);
        let current = make_jit_suite(&[("Fibonacci", 1.8)]); // -28% drop
        let report = compare_jit(&baseline, &current, 20.0);
        assert_eq!(report.status, RegressionStatus::Regression);
        assert_eq!(report.regressions.len(), 1);
        assert!(report.regressions[0].change_percent < -20.0);
    }

    #[test]
    fn test_jit_detects_improvement() {
        let baseline = make_jit_suite(&[("Fibonacci", 2.0)]);
        let current = make_jit_suite(&[("Fibonacci", 3.0)]); // +50%
        let report = compare_jit(&baseline, &current, 20.0);
        assert_eq!(report.status, RegressionStatus::Stable);
        assert!(report.regressions.is_empty());
        assert_eq!(report.improvements.len(), 1);
        assert!(report.improvements[0].change_percent > 20.0);
    }

    #[test]
    fn test_jit_missing_scenario_skipped() {
        let baseline = make_jit_suite(&[("Fibonacci", 2.5)]);
        let current = make_jit_suite(&[("Unknown", 1.0)]);
        let report = compare_jit(&baseline, &current, 20.0);
        assert_eq!(report.status, RegressionStatus::Stable);
        assert!(report.regressions.is_empty());
    }

    #[test]
    fn test_jit_none_speedup_skipped() {
        let baseline = make_jit_suite(&[("Fibonacci", 2.5)]);
        let mut current = make_jit_suite(&[("Fibonacci", 2.5)]);
        current.results[0].speedup = None; // JIT unavailable
        let report = compare_jit(&baseline, &current, 20.0);
        assert_eq!(report.status, RegressionStatus::Stable);
        assert!(report.regressions.is_empty());
    }

    #[test]
    fn test_jit_multi_scenario_worst_wins() {
        let baseline = make_jit_suite(&[("Fibonacci", 2.5), ("BubbleSort", 2.2)]);
        let current = make_jit_suite(&[
            ("Fibonacci", 2.4),  // -4%, within threshold
            ("BubbleSort", 1.5), // -31.8%, regression
        ]);
        let report = compare_jit(&baseline, &current, 20.0);
        assert_eq!(report.status, RegressionStatus::Regression);
        assert_eq!(report.regressions.len(), 1);
        assert_eq!(report.regressions[0].scenario, "BubbleSort");
    }

    #[test]
    fn test_jit_within_threshold_is_stable() {
        let baseline = make_jit_suite(&[("Fibonacci", 2.5)]);
        let current = make_jit_suite(&[("Fibonacci", 2.2)]); // -12%, under 20%
        let report = compare_jit(&baseline, &current, 20.0);
        assert_eq!(report.status, RegressionStatus::Stable);
        assert!(report.regressions.is_empty());
    }
}
