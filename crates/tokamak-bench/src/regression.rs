use crate::types::{BenchSuite, Regression, RegressionReport, RegressionStatus, Thresholds};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BenchResult, OpcodeEntry};

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
}
