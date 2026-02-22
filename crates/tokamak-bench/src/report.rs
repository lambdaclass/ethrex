use crate::types::{BenchSuite, RegressionReport};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BenchResult, OpcodeEntry, RegressionStatus, Thresholds};

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
}
