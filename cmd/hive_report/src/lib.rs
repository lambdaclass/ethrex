use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCase {
    pub summary_result: SummaryResult,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryResult {
    pub pass: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonFile {
    pub name: String,
    pub test_cases: std::collections::HashMap<String, TestCase>,
}

#[derive(Debug)]
pub struct HiveResult {
    pub category: String,
    pub display_name: String,
    pub passed_tests: usize,
    pub total_tests: usize,
    pub success_percentage: f64,
}

impl HiveResult {
    pub fn new(suite: String, passed_tests: usize, total_tests: usize) -> Self {
        let success_percentage = (passed_tests as f64 / total_tests as f64) * 100.0;

        let (category, display_name) = match suite.as_str() {
            "engine-api" => ("Engine", "Paris"),
            "engine-auth" => ("Engine", "Auth"),
            "engine-cancun" => ("Engine", "Cancun"),
            "engine-exchange-capabilities" => ("Engine", "Exchange Capabilities"),
            "engine-withdrawals" => ("Engine", "Shanghai"),
            "discv4" => ("P2P", "Discovery V4"),
            "eth" => ("P2P", "Eth capability"),
            "snap" => ("P2P", "Snap capability"),
            "rpc-compat" => ("RPC", "RPC API Compatibility"),
            "sync" => ("Sync", "Node Syncing"),
            other => {
                eprintln!("Warn: Unknown suite: {}. Skipping", other);
                ("", "")
            }
        };

        HiveResult {
            category: category.to_string(),
            display_name: display_name.to_string(),
            passed_tests,
            total_tests,
            success_percentage,
        }
    }

    pub fn should_skip(&self) -> bool {
        self.category.is_empty()
    }
}

impl std::fmt::Display for HiveResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}/{} ({:.02}%)",
            self.display_name, self.passed_tests, self.total_tests, self.success_percentage
        )
    }
}
