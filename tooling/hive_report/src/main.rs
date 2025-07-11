use serde::Deserialize;
use std::fs::{self, File};
use std::io::BufReader;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestCase {
    summary_result: SummaryResult,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummaryResult {
    pass: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonFile {
    name: String,
    test_cases: std::collections::HashMap<String, TestCase>,
}

struct HiveResult {
    category: String,
    display_name: String,
    passed_tests: usize,
    total_tests: usize,
    success_percentage: f64,
}

impl HiveResult {
    fn new(suite: String, fork: String, passed_tests: usize, total_tests: usize) -> Self {
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
            "eest/consume-rlp" => ("EVM - Consume RLP", fork.as_str()),
            "eest/consume-engine" => ("EVM - Consume Engine", fork.as_str()),
            other => {
                eprintln!("Warn: Unknown suite: {other}. Skipping");
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

    fn should_skip(&self) -> bool {
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

fn create_fork_result(json_data: &JsonFile, fork: &str, test_pattern: &str) -> HiveResult {
    let total_tests = json_data
        .test_cases
        .iter()
        .filter(|(_, test_case)| test_case.name.contains(test_pattern))
        .count();
    let passed_tests = json_data
        .test_cases
        .iter()
        .filter(|(_, test_case)| {
            test_case.name.contains(test_pattern) && test_case.summary_result.pass
        })
        .count();
    HiveResult::new(
        json_data.name.clone(),
        fork.to_string(),
        passed_tests,
        total_tests,
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut results = Vec::new();

    for entry in fs::read_dir("hive/workspace/logs")? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file()
            && path.extension().and_then(|s| s.to_str()) == Some("json")
            && path.file_name().and_then(|s| s.to_str()) != Some("hive.json")
        {
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .expect("Path should be a valid string");
            let file = File::open(&path)?;
            let reader = BufReader::new(file);

            let json_data: JsonFile = match serde_json::from_reader(reader) {
                Ok(data) => data,
                Err(_) => {
                    eprintln!("Error processing file: {file_name}");
                    continue;
                }
            };

            // Both of these simulators have only 1 suite where we can find tests for 3 different forks.
            // To get the total tests and the passed tests a filtes is done each time so we do not clone the test cases each time.
            if json_data.name.as_str() == "eest/consume-rlp"
                || json_data.name.as_str() == "eest/consume-engine"
            {
                let result_paris = create_fork_result(&json_data, "Paris", "fork_Paris");
                // Shanghai
                let result_shanghai = create_fork_result(&json_data, "Shanghai", "fork_Shanghai");
                // Cancun
                let result_cancun = create_fork_result(&json_data, "Cancun", "fork_Cancun");
                // Prague
                let result_prague = create_fork_result(&json_data, "Prague", "fork_Prague");

                results.push(result_paris);
                results.push(result_shanghai);
                results.push(result_cancun);
                results.push(result_prague);
            } else {
                let total_tests = json_data.test_cases.len();
                let passed_tests = json_data
                    .test_cases
                    .values()
                    .filter(|test_case| test_case.summary_result.pass)
                    .count();

                let result =
                    HiveResult::new(json_data.name, String::new(), passed_tests, total_tests);
                if !result.should_skip() {
                    results.push(result);
                }
            }
        }
    }

    // First by category ascending, then by passed tests descending, then by success percentage descending.
    results.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| b.passed_tests.cmp(&a.passed_tests))
            .then_with(|| {
                b.success_percentage
                    .partial_cmp(&a.success_percentage)
                    .unwrap()
            })
    });

    let results_by_category = results.chunk_by(|a, b| a.category == b.category);

    for results in results_by_category {
        // print category
        println!("*{}*", results[0].category);
        for result in results {
            println!("\t{result}");
        }
        println!();
    }

    println!();
    let total_passed = results.iter().map(|r| r.passed_tests).sum::<usize>();
    let total_tests = results.iter().map(|r| r.total_tests).sum::<usize>();
    let total_percentage = (total_passed as f64 / total_tests as f64) * 100.0;
    println!("*Total: {total_passed}/{total_tests} ({total_percentage:.02}%)*");

    Ok(())
}
