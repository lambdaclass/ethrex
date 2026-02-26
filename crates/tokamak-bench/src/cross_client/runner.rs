use std::time::{Duration, Instant};

use bytes::Bytes;
use serde_json::json;
use url::Url;

use crate::runner::{self, Scenario, generate_calldata, load_contract_bytecode};
use crate::stats;

use super::types::*;

/// The contract address used in state overrides (matches in-process bench).
const CONTRACT_ADDRESS: u64 = crate::runner::CONTRACT_ADDRESS;
/// The sender address used in eth_call (matches in-process bench).
const SENDER_ADDRESS: u64 = crate::runner::SENDER_ADDRESS;
/// Gas limit for external eth_call (same as in-process bench).
const GAS_LIMIT: u64 = (i64::MAX - 1) as u64;

/// Send `eth_call` with state overrides to an external client.
///
/// State override injects contract bytecode at `CONTRACT_ADDRESS` (0x42) so the
/// external node does not need the contract deployed on-chain.
async fn eth_call_with_state_override(
    client: &reqwest::Client,
    endpoint: &Url,
    bytecode_hex: &str,
    calldata: &Bytes,
    gas_limit: u64,
) -> Result<Duration, String> {
    let from = format!("0x{SENDER_ADDRESS:040x}");
    let to = format!("0x{CONTRACT_ADDRESS:040x}");
    let data = format!("0x{}", hex::encode(calldata));
    let gas = format!("0x{gas_limit:x}");
    let override_address = format!("0x{CONTRACT_ADDRESS:040x}");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "eth_call",
        "params": [
            {
                "from": from,
                "to": to,
                "data": data,
                "gas": gas
            },
            "latest",
            {
                (override_address): {
                    "code": format!("0x{bytecode_hex}"),
                    "balance": "0xffffffffffffffffffffffffffffffffffffffffffffffffffff"
                }
            }
        ],
        "id": 1
    });

    let start = Instant::now();
    let resp = client
        .post(endpoint.as_str())
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let elapsed = start.elapsed();

    let status = resp.status();
    let resp_body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    if !status.is_success() {
        return Err(format!("HTTP {status}: {resp_body}"));
    }

    if let Some(error) = resp_body.get("error") {
        return Err(format!("RPC error: {error}"));
    }

    Ok(elapsed)
}

/// Run a single scenario against one external client endpoint.
async fn run_scenario_on_client(
    client: &reqwest::Client,
    endpoint: &ClientEndpoint,
    bytecode_hex: &str,
    calldata: &Bytes,
    runs: u64,
    warmup: u64,
    gas_limit: u64,
) -> Result<CrossClientResult, String> {
    let total_runs = warmup + runs;
    let mut durations: Vec<Duration> = Vec::with_capacity(total_runs as usize);

    for _ in 0..total_runs {
        let elapsed =
            eth_call_with_state_override(client, &endpoint.url, bytecode_hex, calldata, gas_limit)
                .await?;
        durations.push(elapsed);
    }

    let measured = stats::split_warmup(&durations, warmup as usize);
    let bench_stats = stats::compute_stats(measured);
    let mean_ns = bench_stats.as_ref().map_or_else(
        || {
            let total: Duration = measured.iter().sum();
            total.as_nanos() as f64 / measured.len() as f64
        },
        |s| s.mean_ns,
    );

    Ok(CrossClientResult {
        client_name: endpoint.name.clone(),
        scenario: String::new(), // filled in by caller
        mean_ns,
        stats: bench_stats,
    })
}

/// Run ethrex in-process for one scenario, returning a `CrossClientResult`.
fn run_ethrex_scenario(
    bytecode_hex: &str,
    iterations: u64,
    runs: u64,
    warmup: u64,
) -> CrossClientResult {
    let result = runner::run_scenario("ethrex", bytecode_hex, runs, iterations, warmup);
    let mean_ns = result.stats.as_ref().map_or_else(
        || result.total_duration_ns as f64 / result.runs as f64,
        |s| s.mean_ns,
    );
    CrossClientResult {
        client_name: "ethrex".to_string(),
        scenario: String::new(),
        mean_ns,
        stats: result.stats,
    }
}

/// Run the full cross-client benchmark suite.
///
/// Executes each scenario first in-process (ethrex) then via `eth_call` against
/// each external endpoint. Returns aggregated results with ethrex as baseline.
pub async fn run_cross_client_suite(
    scenarios: &[Scenario],
    endpoints: &[ClientEndpoint],
    runs: u64,
    warmup: u64,
    commit: &str,
) -> CrossClientSuite {
    let http_client = reqwest::Client::new();
    let mut cross_scenarios = Vec::new();

    for scenario in scenarios {
        let bytecode_hex = match load_contract_bytecode(scenario.name) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Skipping {}: {e}", scenario.name);
                continue;
            }
        };

        let calldata = generate_calldata(scenario.iterations);
        eprintln!("Running {} across clients...", scenario.name);

        // 1. Run ethrex in-process
        let mut ethrex_result =
            run_ethrex_scenario(&bytecode_hex, scenario.iterations, runs, warmup);
        ethrex_result.scenario = scenario.name.to_string();
        let ethrex_mean = ethrex_result.mean_ns;

        let mut results = vec![ethrex_result];

        // 2. Run external clients sequentially
        for endpoint in endpoints {
            eprintln!("  {} @ {}...", endpoint.name, endpoint.url);
            match run_scenario_on_client(
                &http_client,
                endpoint,
                &bytecode_hex,
                &calldata,
                runs,
                warmup,
                GAS_LIMIT,
            )
            .await
            {
                Ok(mut r) => {
                    r.scenario = scenario.name.to_string();
                    results.push(r);
                }
                Err(e) => {
                    eprintln!("  Error for {} on {}: {e}", scenario.name, endpoint.name);
                }
            }
        }

        cross_scenarios.push(CrossClientScenario {
            scenario: scenario.name.to_string(),
            ethrex_mean_ns: ethrex_mean,
            results,
        });
    }

    CrossClientSuite {
        timestamp: unix_timestamp_secs(),
        commit: commit.to_string(),
        scenarios: cross_scenarios,
    }
}

fn unix_timestamp_secs() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_limit_value() {
        // Same as in-process bench: (i64::MAX - 1)
        assert_eq!(GAS_LIMIT, (i64::MAX - 1) as u64);
    }

    #[test]
    fn test_addresses_match_runner() {
        assert_eq!(CONTRACT_ADDRESS, 0x42);
        assert_eq!(SENDER_ADDRESS, 0x100);
    }

    #[test]
    fn test_unix_timestamp() {
        let ts = unix_timestamp_secs();
        let secs: u64 = ts.parse().expect("should be a number");
        // Should be a reasonable recent timestamp (after 2024)
        assert!(secs > 1_700_000_000);
    }

    #[test]
    fn test_run_ethrex_scenario() {
        // Uses the "Push" scenario which has 0 iterations (simplest)
        let bytecode_hex = match crate::runner::load_contract_bytecode("Push") {
            Ok(b) => b,
            Err(_) => return, // skip if contract not available
        };
        let result = run_ethrex_scenario(&bytecode_hex, 0, 3, 1);
        assert_eq!(result.client_name, "ethrex");
        assert!(result.mean_ns > 0.0);
    }
}
