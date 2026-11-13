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
            "discv5" => ("P2P", "Discovery V5"),
            "eth" => ("P2P", "Eth capability"),
            "snap" => ("P2P", "Snap capability"),
            "snap2" => ("P2P", "Snap/2 capability"),
            "rpc-compat" => ("RPC", "RPC API Compatibility"),
            "sync" => ("Sync", "Node Syncing"),
            "snapsync" => ("Sync", "Snap Syncing"),
            "eels/consume-rlp" => ("EVM - Consume RLP", fork.as_str()),
            "eels/consume-engine" => ("EVM - Consume Engine", fork.as_str()),
            "eels/consume-wirex" => ("EVM - Consume WireX", fork.as_str()),
            // The name the simulator registers its suite under, which is not the
            // `--sim` name `eels/execute-blobs`; matching the latter silently
            // dropped every execute run as an unknown suite.
            "eels/execute, hive mode" => ("EVM - Execute Blobs", fork.as_str()),
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

/// Suites of the EELS simulators, which hold the tests of every fork in one suite.
const FORK_SPLIT_SUITES: [&str; 4] = [
    "eels/consume-rlp",
    "eels/consume-engine",
    "eels/consume-wirex",
    // Runs once per fork (see daily_hive_report.yaml). Without the split both
    // runs land in the same row and a fork-specific regression is invisible.
    "eels/execute, hive mode",
];

/// Row for the test cases whose id names no fork, which are the simulator's own
/// harness cases, such as its client launch.
const NO_FORK: &str = "No fork";

/// The fork a test case ran on, read from the `fork_<Name>` parameter of its id,
/// as in `test_x.py::test_x[fork_Prague-blockchain_test_engine-case]`.
///
/// Only the parameter list is searched, so a `fork_` in a file or test name is
/// not taken for one. The name is returned whole: `BPO2ToAmsterdamAtTime15k` is
/// its own fork, not a `BPO2` or an `Amsterdam` case.
fn fork_of(test_name: &str) -> Option<&str> {
    let (_, params) = test_name.split_once('[')?;
    params
        .split(['-', ']'])
        .find_map(|param| param.strip_prefix("fork_"))
        .filter(|fork| !fork.is_empty())
}

/// One result per fork found in the suite, so every test case lands in a row.
fn fork_results(json_data: &JsonFile) -> Vec<HiveResult> {
    let mut per_fork: HashMap<&str, (usize, usize)> = HashMap::new();
    for test_case in json_data.test_cases.values() {
        let (passed, total) = per_fork
            .entry(fork_of(&test_case.name).unwrap_or(NO_FORK))
            .or_insert((0, 0));
        *total += 1;
        if test_case.summary_result.pass {
            *passed += 1;
        }
    }
    per_fork
        .into_iter()
        .map(|(fork, (passed_tests, total_tests))| {
            HiveResult::new(
                json_data.name.clone(),
                fork.to_string(),
                passed_tests,
                total_tests,
            )
        })
        .collect()
}

/// Forks from newest to oldest, the order their rows are listed in.
const FORK_ORDER: [&str; 8] = [
    "Amsterdam",
    "BPO2",
    "BPO1",
    "Osaka",
    "Prague",
    "Cancun",
    "Shanghai",
    "Paris",
];

/// Sort key that lists a fork's own row first and the forks built on it, such
/// as `OsakaToBPO1AtTime15k`, right after it. `None` for a name that starts
/// with no known fork.
fn fork_rank(display_name: &str) -> Option<(usize, bool, &str)> {
    FORK_ORDER
        .iter()
        .position(|fork| display_name.starts_with(fork))
        .map(|index| (index, display_name != FORK_ORDER[index], display_name))
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

            if FORK_SPLIT_SUITES.contains(&json_data.name.as_str()) {
                for result in fork_results(&json_data) {
                    aggregate_result(&mut aggregated_results, result);
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn suite(name: &str, cases: &[(&str, bool)]) -> JsonFile {
        JsonFile {
            name: name.to_string(),
            test_cases: cases
                .iter()
                .enumerate()
                .map(|(index, (case, pass))| {
                    (
                        index.to_string(),
                        TestCase {
                            summary_result: SummaryResult { pass: *pass },
                            name: case.to_string(),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn fork_is_read_from_the_parameter_list() {
        assert_eq!(
            fork_of("tests/prague/x.py::test_x[fork_Prague-blockchain_test_engine-case]-ethrex"),
            Some("Prague")
        );
        assert_eq!(
            fork_of("t.py::test_t[fork_BPO2ToAmsterdamAtTime15k-blockchain_test]"),
            Some("BPO2ToAmsterdamAtTime15k")
        );
        assert_eq!(fork_of("t.py::test_t[case_1-fork_Osaka]"), Some("Osaka"));
    }

    #[test]
    fn a_fork_in_a_file_or_test_name_is_not_taken_for_one() {
        assert_eq!(
            fork_of("tests/x/test_fork_transition.py::test_fork_transition[case_1]"),
            None
        );
        assert_eq!(fork_of("a test without parameters"), None);
        assert_eq!(fork_of("t.py::test_t[fork_-case]"), None);
    }

    #[test]
    fn every_case_lands_in_exactly_one_row() {
        let json = suite(
            "eels/consume-engine",
            &[
                ("t.py::a[fork_Amsterdam-x]", true),
                ("t.py::b[fork_Amsterdam-x]", false),
                ("t.py::c[fork_AmsterdamBPOIncrease-x]", true),
                ("t.py::d[fork_BPO2ToAmsterdamAtTime15k-x]", true),
                ("t.py::e[fork_BPO1-x]", true),
                ("t.py::f[fork_Osaka-x]", true),
                ("t.py::g[no_fork_here]", false),
            ],
        );
        let mut rows: Vec<(String, usize, usize)> = fork_results(&json)
            .into_iter()
            .map(|r| (r.display_name, r.passed_tests, r.total_tests))
            .collect();
        rows.sort();
        assert_eq!(
            rows,
            vec![
                ("Amsterdam".to_string(), 1, 2),
                ("AmsterdamBPOIncrease".to_string(), 1, 1),
                ("BPO1".to_string(), 1, 1),
                ("BPO2ToAmsterdamAtTime15k".to_string(), 1, 1),
                (NO_FORK.to_string(), 0, 1),
                ("Osaka".to_string(), 1, 1),
            ]
        );
        let total: usize = rows.iter().map(|(_, _, total)| total).sum();
        assert_eq!(total, json.test_cases.len());
    }

    #[test]
    fn forks_are_listed_newest_first_and_a_fork_before_the_forks_built_on_it() {
        let mut names = vec![
            "Paris",
            "OsakaToBPO1AtTime15k",
            "BPO1",
            "Amsterdam",
            "Osaka",
            "AmsterdamBPOIncrease",
            "BPO2ToAmsterdamAtTime15k",
            "BPO2",
        ];
        names.sort_by_key(|name| fork_rank(name));
        assert_eq!(
            names,
            vec![
                "Amsterdam",
                "AmsterdamBPOIncrease",
                "BPO2",
                "BPO2ToAmsterdamAtTime15k",
                "BPO1",
                "Osaka",
                "OsakaToBPO1AtTime15k",
                "Paris",
            ]
        );
        assert_eq!(fork_rank("Auth"), None);
    }
}
