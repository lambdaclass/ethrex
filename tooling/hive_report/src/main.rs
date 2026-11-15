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
/// harness cases, such as its client launch. The report keeps them so that every
/// case is in a row; the check that a run executed tests,
/// `.github/scripts/count-hive-tests.sh`, leaves them out.
const NO_FORK: &str = "No fork";

/// The fork a test case ran on, read from the `fork_<Name>` parameter of its id,
/// as in `test_x.py::test_x[fork_Prague-blockchain_test_engine-case]`.
///
/// Only the parameter list is searched, from the first `[` to the last `]`, so
/// a `fork_` in a file or test name, or after the list, is not taken for one.
/// The name is returned whole: `BPO2ToAmsterdamAtTime15k` is its own fork, not
/// a `BPO2` or an `Amsterdam` case.
fn fork_of(test_name: &str) -> Option<&str> {
    let (_, rest) = test_name.split_once('[')?;
    let (params, _) = rest.rsplit_once(']')?;
    params
        .split(['-', '[', ']'])
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

/// Forks from newest to oldest, the order their rows are listed in. The daily
/// jobs select the forks from Paris on. A fork that is not here, an older one
/// or one this list has not learned yet, is still reported, after these.
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

/// Sort key that lists a fork's own row first and, right after it, the rows
/// whose name starts with that fork: the transitions out of it, such as
/// `OsakaToBPO1AtTime15k`, and its variants, such as `AmsterdamBPOIncrease`.
/// `None` for a name that starts with no known fork.
///
/// A digit after the fork's name makes it another fork, so `BPO10` is not
/// ranked under `BPO1`.
fn fork_rank(display_name: &str) -> Option<(usize, bool, &str)> {
    FORK_ORDER
        .iter()
        .position(|fork| {
            display_name
                .strip_prefix(fork)
                .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_digit()))
        })
        .map(|index| (index, display_name != FORK_ORDER[index], display_name))
}

/// Order of the report's rows: by category, then the fork rows from newest to
/// oldest, then the rows that name no known fork, the one with most passed
/// tests first.
///
/// A row with a fork rank sorts before every row without one. Comparing by
/// rank only when both rows have one, and by passed tests otherwise, is not a
/// total order: it can put A before B, B before C and C before A, and
/// `sort_by` may panic on such a comparison.
fn compare_rows(a: &HiveResult, b: &HiveResult) -> Ordering {
    a.category
        .cmp(&b.category)
        .then_with(
            || match (fork_rank(&a.display_name), fork_rank(&b.display_name)) {
                (Some(rank_a), Some(rank_b)) => rank_a.cmp(&rank_b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        )
        .then_with(|| b.passed_tests.cmp(&a.passed_tests))
        .then_with(|| b.success_percentage.total_cmp(&a.success_percentage))
        .then_with(|| a.display_name.cmp(&b.display_name))
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

    results.sort_by(compare_rows);

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
        assert_eq!(fork_of("t.py::test_t[fork_Prague][extra]"), Some("Prague"));
    }

    #[test]
    fn a_fork_in_a_file_or_test_name_is_not_taken_for_one() {
        assert_eq!(
            fork_of("tests/x/test_fork_transition.py::test_fork_transition[case_1]"),
            None
        );
        assert_eq!(fork_of("a test without parameters"), None);
        assert_eq!(fork_of("t.py::test_t[fork_-case]"), None);
        assert_eq!(fork_of("t.py::test_t[case_1]-fork_Osaka"), None);
        assert_eq!(fork_of("t.py::test_t[fork_Osaka-case_1"), None);
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
        assert_eq!(fork_rank("BPO10"), None);
        assert_eq!(fork_rank("BPO1"), Some((2, false, "BPO1")));
    }

    fn row(category: &str, display_name: &str, passed_tests: usize) -> HiveResult {
        HiveResult {
            category: category.to_string(),
            display_name: display_name.to_string(),
            passed_tests,
            total_tests: passed_tests,
            success_percentage: 100.0,
        }
    }

    #[test]
    fn rows_without_a_fork_come_after_the_fork_rows_whatever_they_passed() {
        // Ranking only pairs of fork rows and comparing passed tests otherwise
        // made a cycle of these three: Amsterdam before Osaka by rank, Osaka
        // before "No fork" by passed tests, "No fork" before Amsterdam by
        // passed tests.
        let rows = [
            row("EVM - Consume Engine", "Amsterdam", 1),
            row("EVM - Consume Engine", "Osaka", 3),
            row("EVM - Consume Engine", NO_FORK, 2),
            row("Engine", "Auth", 9),
            row("Engine", "Paris", 1),
            row("Engine", "Exchange Capabilities", 5),
        ];
        let expected = [
            "Amsterdam",
            "Osaka",
            NO_FORK,
            "Paris",
            "Auth",
            "Exchange Capabilities",
        ];
        // Every starting order has to give the same result.
        for rotation in 0..rows.len() {
            let mut sorted: Vec<&HiveResult> = rows
                .iter()
                .cycle()
                .skip(rotation)
                .take(rows.len())
                .collect();
            sorted.sort_by(|a, b| compare_rows(a, b));
            let names: Vec<&str> = sorted.iter().map(|r| r.display_name.as_str()).collect();
            assert_eq!(names, expected);
        }
        for a in &rows {
            for b in &rows {
                assert_eq!(compare_rows(a, b), compare_rows(b, a).reverse());
            }
        }
    }
}
