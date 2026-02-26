use super::types::CrossClientSuite;

/// Serialize a `CrossClientSuite` to pretty-printed JSON.
pub fn to_json(suite: &CrossClientSuite) -> String {
    serde_json::to_string_pretty(suite).expect("Failed to serialize CrossClientSuite")
}

/// Deserialize a `CrossClientSuite` from JSON.
pub fn from_json(json: &str) -> CrossClientSuite {
    serde_json::from_str(json).expect("Failed to deserialize CrossClientSuite")
}

/// Generate a markdown comparison table with ethrex as 1.00x baseline.
///
/// For each scenario, shows the mean execution time (ms) per client and a
/// relative speedup ratio where ethrex = 1.00x.
pub fn to_markdown(suite: &CrossClientSuite) -> String {
    let mut md = String::new();

    md.push_str("## Cross-Client Benchmark Comparison\n\n");
    md.push_str(&format!("Commit: `{}`\n\n", suite.commit));

    if suite.scenarios.is_empty() {
        md.push_str("No scenarios were executed.\n");
        return md;
    }

    // Collect all unique client names (preserving order, ethrex first)
    let client_names: Vec<String> = {
        let mut names = Vec::new();
        for scenario in &suite.scenarios {
            for result in &scenario.results {
                if !names.contains(&result.client_name) {
                    names.push(result.client_name.clone());
                }
            }
        }
        names
    };

    // Header row
    md.push_str("| Scenario ");
    for name in &client_names {
        md.push_str(&format!("| {name} (ms) | {name} ratio "));
    }
    md.push_str("|\n");

    // Separator row
    md.push_str("|----------");
    for _ in &client_names {
        md.push_str("|----------:|----------:");
    }
    md.push_str("|\n");

    // Data rows
    for scenario in &suite.scenarios {
        md.push_str(&format!("| {} ", scenario.scenario));
        let baseline = scenario.ethrex_mean_ns;

        for name in &client_names {
            if let Some(result) = scenario.results.iter().find(|r| r.client_name == *name) {
                let ms = result.mean_ns / 1_000_000.0;
                let ratio = if baseline > 0.0 {
                    result.mean_ns / baseline
                } else {
                    f64::NAN
                };
                md.push_str(&format!("| {ms:.3} | {ratio:.2}x "));
            } else {
                md.push_str("| N/A | N/A ");
            }
        }
        md.push_str("|\n");
    }

    md.push('\n');
    md.push_str("*Ratio: relative to ethrex (1.00x = same speed, >1.00x = slower than ethrex)*\n");
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_client::types::*;
    use crate::stats::BenchStats;

    fn sample_suite() -> CrossClientSuite {
        CrossClientSuite {
            timestamp: "1700000000".to_string(),
            commit: "abc123".to_string(),
            scenarios: vec![
                CrossClientScenario {
                    scenario: "Fibonacci".to_string(),
                    ethrex_mean_ns: 1_000_000.0,
                    results: vec![
                        CrossClientResult {
                            client_name: "ethrex".to_string(),
                            scenario: "Fibonacci".to_string(),
                            mean_ns: 1_000_000.0,
                            stats: None,
                        },
                        CrossClientResult {
                            client_name: "geth".to_string(),
                            scenario: "Fibonacci".to_string(),
                            mean_ns: 2_500_000.0,
                            stats: None,
                        },
                        CrossClientResult {
                            client_name: "reth".to_string(),
                            scenario: "Fibonacci".to_string(),
                            mean_ns: 1_800_000.0,
                            stats: None,
                        },
                    ],
                },
                CrossClientScenario {
                    scenario: "BubbleSort".to_string(),
                    ethrex_mean_ns: 5_000_000.0,
                    results: vec![
                        CrossClientResult {
                            client_name: "ethrex".to_string(),
                            scenario: "BubbleSort".to_string(),
                            mean_ns: 5_000_000.0,
                            stats: None,
                        },
                        CrossClientResult {
                            client_name: "geth".to_string(),
                            scenario: "BubbleSort".to_string(),
                            mean_ns: 4_000_000.0,
                            stats: None,
                        },
                    ],
                },
            ],
        }
    }

    #[test]
    fn test_json_roundtrip() {
        let suite = sample_suite();
        let json = to_json(&suite);
        let parsed = from_json(&json);
        assert_eq!(parsed.commit, "abc123");
        assert_eq!(parsed.scenarios.len(), 2);
        assert_eq!(parsed.scenarios[0].results.len(), 3);
    }

    #[test]
    fn test_markdown_contains_header() {
        let suite = sample_suite();
        let md = to_markdown(&suite);
        assert!(md.contains("Cross-Client Benchmark Comparison"));
        assert!(md.contains("abc123"));
    }

    #[test]
    fn test_markdown_contains_clients() {
        let suite = sample_suite();
        let md = to_markdown(&suite);
        assert!(md.contains("ethrex (ms)"));
        assert!(md.contains("geth (ms)"));
        assert!(md.contains("reth (ms)"));
    }

    #[test]
    fn test_markdown_ethrex_ratio_is_one() {
        let suite = sample_suite();
        let md = to_markdown(&suite);
        // ethrex ratio should be 1.00x
        assert!(md.contains("1.00x"));
    }

    #[test]
    fn test_markdown_geth_ratio() {
        let suite = sample_suite();
        let md = to_markdown(&suite);
        // Fibonacci: geth 2.5M / ethrex 1M = 2.50x
        assert!(md.contains("2.50x"));
    }

    #[test]
    fn test_markdown_faster_than_ethrex() {
        let suite = sample_suite();
        let md = to_markdown(&suite);
        // BubbleSort: geth 4M / ethrex 5M = 0.80x
        assert!(md.contains("0.80x"));
    }

    #[test]
    fn test_markdown_empty_suite() {
        let suite = CrossClientSuite {
            timestamp: "0".to_string(),
            commit: "empty".to_string(),
            scenarios: vec![],
        };
        let md = to_markdown(&suite);
        assert!(md.contains("No scenarios"));
    }

    #[test]
    fn test_markdown_missing_client() {
        let suite = sample_suite();
        let md = to_markdown(&suite);
        // BubbleSort has no reth entry — should show N/A
        assert!(md.contains("N/A"));
    }

    #[test]
    fn test_markdown_footer() {
        let suite = sample_suite();
        let md = to_markdown(&suite);
        assert!(md.contains("Ratio: relative to ethrex"));
    }

    #[test]
    fn test_json_roundtrip_with_stats() {
        let suite = CrossClientSuite {
            timestamp: "123".to_string(),
            commit: "def456".to_string(),
            scenarios: vec![CrossClientScenario {
                scenario: "Test".to_string(),
                ethrex_mean_ns: 100.0,
                results: vec![CrossClientResult {
                    client_name: "ethrex".to_string(),
                    scenario: "Test".to_string(),
                    mean_ns: 100.0,
                    stats: Some(BenchStats {
                        mean_ns: 100.0,
                        stddev_ns: 10.0,
                        ci_lower_ns: 90.0,
                        ci_upper_ns: 110.0,
                        min_ns: 80,
                        max_ns: 120,
                        samples: 5,
                    }),
                }],
            }],
        };
        let json = to_json(&suite);
        let parsed = from_json(&json);
        assert!(parsed.scenarios[0].results[0].stats.is_some());
    }
}
