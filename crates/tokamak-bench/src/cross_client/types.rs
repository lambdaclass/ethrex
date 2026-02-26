use serde::{Deserialize, Serialize};
use url::Url;

use crate::stats::BenchStats;

/// A named RPC endpoint for an external EVM client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientEndpoint {
    /// Human-readable client name (e.g. "geth", "reth").
    pub name: String,
    /// JSON-RPC URL.
    pub url: Url,
}

/// Benchmark result for a single client on a single scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossClientResult {
    /// Client name (e.g. "ethrex", "geth", "reth").
    pub client_name: String,
    /// Scenario name (e.g. "Fibonacci").
    pub scenario: String,
    /// Mean execution time in nanoseconds.
    pub mean_ns: f64,
    /// Statistical summary (None if < 2 samples).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<BenchStats>,
}

/// Aggregated results for a single scenario across all clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossClientScenario {
    /// Scenario name.
    pub scenario: String,
    /// Per-client results for this scenario.
    pub results: Vec<CrossClientResult>,
    /// Ethrex mean (ns) used as the 1.00x baseline.
    pub ethrex_mean_ns: f64,
}

/// Full cross-client benchmark suite with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossClientSuite {
    /// Unix timestamp of the benchmark run.
    pub timestamp: String,
    /// Git commit hash.
    pub commit: String,
    /// Per-scenario results.
    pub scenarios: Vec<CrossClientScenario>,
}

/// Parse an endpoints string like "geth=http://localhost:8546,reth=http://localhost:8547"
/// into a list of `ClientEndpoint`.
pub fn parse_endpoints(input: &str) -> Result<Vec<ClientEndpoint>, String> {
    let mut endpoints = Vec::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, url_str) = part
            .split_once('=')
            .ok_or_else(|| format!("Invalid endpoint format: '{part}' (expected name=url)"))?;
        let name = name.trim().to_string();
        let url =
            Url::parse(url_str.trim()).map_err(|e| format!("Invalid URL for '{name}': {e}"))?;
        endpoints.push(ClientEndpoint { name, url });
    }
    if endpoints.is_empty() {
        return Err("No endpoints provided".to_string());
    }
    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cross_client_result_serialization() {
        let result = CrossClientResult {
            client_name: "geth".to_string(),
            scenario: "Fibonacci".to_string(),
            mean_ns: 1_500_000.0,
            stats: None,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let parsed: CrossClientResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.client_name, "geth");
        assert_eq!(parsed.scenario, "Fibonacci");
        assert!((parsed.mean_ns - 1_500_000.0).abs() < 0.1);
        assert!(parsed.stats.is_none());
    }

    #[test]
    fn test_cross_client_result_with_stats() {
        use crate::stats::BenchStats;

        let result = CrossClientResult {
            client_name: "reth".to_string(),
            scenario: "BubbleSort".to_string(),
            mean_ns: 3_000_000.0,
            stats: Some(BenchStats {
                mean_ns: 3_000_000.0,
                stddev_ns: 100_000.0,
                ci_lower_ns: 2_900_000.0,
                ci_upper_ns: 3_100_000.0,
                min_ns: 2_800_000,
                max_ns: 3_200_000,
                samples: 10,
            }),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let parsed: CrossClientResult = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.stats.is_some());
        assert_eq!(parsed.stats.as_ref().unwrap().samples, 10);
    }

    #[test]
    fn test_cross_client_scenario_serialization() {
        let scenario = CrossClientScenario {
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
                    mean_ns: 2_000_000.0,
                    stats: None,
                },
            ],
        };
        let json = serde_json::to_string(&scenario).expect("serialize");
        let parsed: CrossClientScenario = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.results.len(), 2);
        assert!((parsed.ethrex_mean_ns - 1_000_000.0).abs() < 0.1);
    }

    #[test]
    fn test_cross_client_suite_roundtrip() {
        let suite = CrossClientSuite {
            timestamp: "1700000000".to_string(),
            commit: "abc123".to_string(),
            scenarios: vec![CrossClientScenario {
                scenario: "Fibonacci".to_string(),
                ethrex_mean_ns: 1_000_000.0,
                results: vec![CrossClientResult {
                    client_name: "ethrex".to_string(),
                    scenario: "Fibonacci".to_string(),
                    mean_ns: 1_000_000.0,
                    stats: None,
                }],
            }],
        };
        let json = serde_json::to_string_pretty(&suite).expect("serialize");
        let parsed: CrossClientSuite = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.commit, "abc123");
        assert_eq!(parsed.scenarios.len(), 1);
    }

    #[test]
    fn test_client_endpoint_serialization() {
        let ep = ClientEndpoint {
            name: "geth".to_string(),
            url: Url::parse("http://localhost:8545").unwrap(),
        };
        let json = serde_json::to_string(&ep).expect("serialize");
        let parsed: ClientEndpoint = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.name, "geth");
        assert_eq!(parsed.url.as_str(), "http://localhost:8545/");
    }

    #[test]
    fn test_parse_endpoints_single() {
        let eps = parse_endpoints("geth=http://localhost:8545").unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].name, "geth");
        assert_eq!(eps[0].url.as_str(), "http://localhost:8545/");
    }

    #[test]
    fn test_parse_endpoints_multiple() {
        let eps = parse_endpoints("geth=http://localhost:8546,reth=http://localhost:8547").unwrap();
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].name, "geth");
        assert_eq!(eps[1].name, "reth");
    }

    #[test]
    fn test_parse_endpoints_with_spaces() {
        let eps = parse_endpoints(" geth = http://localhost:8546 , reth = http://localhost:8547 ")
            .unwrap();
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].name, "geth");
        assert_eq!(eps[1].name, "reth");
    }

    #[test]
    fn test_parse_endpoints_invalid_format() {
        let err = parse_endpoints("geth-http://localhost:8545").unwrap_err();
        assert!(err.contains("expected name=url"));
    }

    #[test]
    fn test_parse_endpoints_invalid_url() {
        let err = parse_endpoints("geth=not-a-url").unwrap_err();
        assert!(err.contains("Invalid URL"));
    }

    #[test]
    fn test_parse_endpoints_empty() {
        let err = parse_endpoints("").unwrap_err();
        assert!(err.contains("No endpoints"));
    }

    #[test]
    fn test_stats_none_skipped_in_json() {
        let result = CrossClientResult {
            client_name: "ethrex".to_string(),
            scenario: "Test".to_string(),
            mean_ns: 100.0,
            stats: None,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(!json.contains("stats"));
    }
}
