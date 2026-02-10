use serde::Deserialize;
use serde_json::json;
use std::cmp::Ordering;
use std::collections::HashMap;
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

const HIVE_SLACK_BLOCKS_FILE_PATH: &str = "./hive_slack_blocks.json";

struct HiveResult {
    category: String,
    display_name: String,
    passed_tests: usize,
    total_tests: usize,
    success_percentage: f64,
}

struct CategoryResults {
    name: String,
    tests: Vec<HiveResult>,
}

impl CategoryResults {
    fn total_passed(&self) -> usize {
        self.tests.iter().map(|res| res.passed_tests).sum()
    }

    fn total_tests(&self) -> usize {
        self.tests.iter().map(|res| res.total_tests).sum()
    }

    fn success_percentage(&self) -> f64 {
        calculate_success_percentage(self.total_passed(), self.total_tests())
    }
}

impl HiveResult {
    fn new(suite: String, fork: String, passed_tests: usize, total_tests: usize) -> Self {
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
            "eels/consume-rlp" => ("EVM - Consume RLP", fork.as_str()),
            "eels/consume-engine" => ("EVM - Consume Engine", fork.as_str()),
            "eels/execute-blobs" => ("EVM - Execute Blobs", "Execute Blobs"),
            other => {
                eprintln!("Warn: Unknown suite: {other}. Skipping");
                ("", "")
            }
        };

        let success_percentage = calculate_success_percentage(passed_tests, total_tests);

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

fn calculate_success_percentage(passed_tests: usize, total_tests: usize) -> f64 {
    if total_tests == 0 {
        0.0
    } else {
        (passed_tests as f64 / total_tests as f64) * 100.0
    }
}

fn build_slack_blocks(
    categories: &[CategoryResults],
    total_passed: usize,
    total_tests: usize,
) -> serde_json::Value {
    let total_percentage = calculate_success_percentage(total_passed, total_tests);
    let mut blocks = vec![json!({
        "type": "header",
        "text": {
            "type": "plain_text",
            "text": format!(
                "Daily Hive Coverage report — {total_passed}/{total_tests} ({total_percentage:.02}%)"
            )
        }
    })];

    for category in categories {
        let category_passed = category.total_passed();
        let category_total = category.total_tests();
        let category_percentage = category.success_percentage();
        let status = if category_passed == category_total {
            "✅"
        } else {
            "⚠️"
        };

        let mut lines = vec![format!(
            "*{}* {}/{} ({:.02}%) {}",
            category.name, category_passed, category_total, category_percentage, status
        )];

        let mut failing_tests: Vec<_> = category
            .tests
            .iter()
            .filter(|result| result.passed_tests < result.total_tests)
            .collect();

        failing_tests.sort_by(|a, b| {
            a.success_percentage
                .partial_cmp(&b.success_percentage)
                .unwrap_or(Ordering::Equal)
        });

        for result in failing_tests {
            lines.push(format!(
                "- {}: {}/{} ({:.02}%)",
                result.display_name,
                result.passed_tests,
                result.total_tests,
                result.success_percentage
            ));
        }

        blocks.push(json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": lines.join("\n"),
            },
        }));
    }

    json!({ "blocks": blocks })
}

fn aggregate_result(
    aggregated_results: &mut HashMap<(String, String), (usize, usize)>,
    result: HiveResult,
) {
    if result.should_skip() {
        return;
    }

    let HiveResult {
        category,
        display_name,
        passed_tests,
        total_tests,
        ..
    } = result;

    let entry = aggregated_results
        .entry((category, display_name))
        .or_insert((0, 0));
    entry.0 += passed_tests;
    entry.1 += total_tests;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut aggregated_results: HashMap<(String, String), (usize, usize)> = HashMap::new();

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
            if json_data.name.as_str() == "eels/consume-rlp"
                || json_data.name.as_str() == "eels/consume-engine"
            {
                let result_paris = create_fork_result(&json_data, "Paris", "fork_Paris");
                // Shanghai
                let result_shanghai = create_fork_result(&json_data, "Shanghai", "fork_Shanghai");
                // Cancun
                let result_cancun = create_fork_result(&json_data, "Cancun", "fork_Cancun");
                // Prague
                let result_prague = create_fork_result(&json_data, "Prague", "fork_Prague");

                let result_osaka = create_fork_result(&json_data, "Osaka", "fork_Osaka");

                let result_amsterdam =
                    create_fork_result(&json_data, "Amsterdam", "fork_Amsterdam");

                aggregate_result(&mut aggregated_results, result_paris);
                aggregate_result(&mut aggregated_results, result_shanghai);
                aggregate_result(&mut aggregated_results, result_cancun);
                aggregate_result(&mut aggregated_results, result_prague);
                aggregate_result(&mut aggregated_results, result_osaka);
                aggregate_result(&mut aggregated_results, result_amsterdam);
            } else {
                let total_tests = json_data.test_cases.len();
                let passed_tests = json_data
                    .test_cases
                    .values()
                    .filter(|test_case| test_case.summary_result.pass)
                    .count();

                let result =
                    HiveResult::new(json_data.name, String::new(), passed_tests, total_tests);
                aggregate_result(&mut aggregated_results, result);
            }
        }
    }

    let mut results: Vec<HiveResult> = aggregated_results
        .into_iter()
        .map(
            |((category, display_name), (passed_tests, total_tests))| HiveResult {
                category,
                display_name,
                passed_tests,
                total_tests,
                success_percentage: calculate_success_percentage(passed_tests, total_tests),
            },
        )
        .collect();

    // First by category ascending, use fork ordering newest → oldest when applicable, then by passed tests descending.
    results.sort_by(|a, b| {
        let category_cmp = a.category.cmp(&b.category);
        if category_cmp != Ordering::Equal {
            return category_cmp;
        }

        let fork_rank = |display_name: &str| match display_name {
            "Amsterdam" => Some(0),
            "Osaka" => Some(1),
            "Prague" => Some(2),
            "Cancun" => Some(3),
            "Shanghai" => Some(4),
            "Paris" => Some(5),
            _ => None,
        };

        if let (Some(rank_a), Some(rank_b)) =
            (fork_rank(&a.display_name), fork_rank(&b.display_name))
        {
            let order_cmp = rank_a.cmp(&rank_b);
            if order_cmp != Ordering::Equal {
                return order_cmp;
            }
        }

        b.passed_tests.cmp(&a.passed_tests).then_with(|| {
            b.success_percentage
                .partial_cmp(&a.success_percentage)
                .unwrap()
        })
    });

    let mut grouped_results: Vec<CategoryResults> = Vec::new();
    for result in results {
        if let Some(last) = grouped_results
            .last_mut()
            .filter(|last| last.name == result.category)
        {
            last.tests.push(result);
            continue;
        }

        let name = result.category.clone();
        grouped_results.push(CategoryResults {
            name,
            tests: vec![result],
        });
    }

    for category in &grouped_results {
        println!("*{}*", category.name);
        for result in &category.tests {
            println!("\t{result}");
        }
        println!();
    }

    println!();
    let total_passed = grouped_results
        .iter()
        .flat_map(|group| group.tests.iter().map(|r| r.passed_tests))
        .sum::<usize>();
    let total_tests = grouped_results
        .iter()
        .flat_map(|group| group.tests.iter().map(|r| r.total_tests))
        .sum::<usize>();
    let total_percentage = calculate_success_percentage(total_passed, total_tests);
    println!("*Total: {total_passed}/{total_tests} ({total_percentage:.02}%)*");

    let slack_blocks = build_slack_blocks(&grouped_results, total_passed, total_tests);
    fs::write(
        HIVE_SLACK_BLOCKS_FILE_PATH,
        serde_json::to_string_pretty(&slack_blocks)?,
    )?;

    Ok(())
}
