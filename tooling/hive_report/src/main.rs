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

// --- Exclusion config ---

#[derive(Debug, Deserialize, Default)]
struct ExclusionConfig {
    #[serde(default)]
    known_issues: Vec<ExclusionRule>,
    #[serde(default)]
    wip: Vec<ExclusionRule>,
}

#[derive(Debug, Deserialize)]
struct ExclusionRule {
    category: String,
    subcategory: String,
    reason: String,
    tests: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ExclusionKind {
    KnownIssue,
    Wip,
}

impl ExclusionConfig {
    fn load() -> Self {
        let config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/exclusions.toml");
        match fs::read_to_string(config_path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Warn: Failed to parse {config_path}: {e}. Using empty config.");
                    Self::default()
                }
            },
            Err(_) => {
                eprintln!("Info: No exclusions.toml found at {config_path}. No exclusions applied.");
                Self::default()
            }
        }
    }

    /// Check if a test case matches any exclusion rule.
    /// Returns the kind and index of the matching rule, if any.
    fn classify(
        &self,
        category: &str,
        subcategory: &str,
        test_name: &str,
        passes: bool,
    ) -> Option<(ExclusionKind, usize)> {
        for (i, rule) in self.known_issues.iter().enumerate() {
            if rule.matches(category, subcategory, test_name, passes) {
                return Some((ExclusionKind::KnownIssue, i));
            }
        }
        for (i, rule) in self.wip.iter().enumerate() {
            if rule.matches(category, subcategory, test_name, passes) {
                return Some((ExclusionKind::Wip, i));
            }
        }
        None
    }

    fn rule(&self, kind: ExclusionKind, index: usize) -> &ExclusionRule {
        match kind {
            ExclusionKind::KnownIssue => &self.known_issues[index],
            ExclusionKind::Wip => &self.wip[index],
        }
    }
}

impl ExclusionRule {
    fn matches(&self, category: &str, subcategory: &str, test_name: &str, passes: bool) -> bool {
        if self.category != category || self.subcategory != subcategory {
            return false;
        }
        match &self.tests {
            // Specific test names listed: exclude those tests regardless of pass/fail
            Some(patterns) => patterns.iter().any(|p| test_name.contains(p.as_str())),
            // No specific tests: only exclude failing tests in this scope
            None => !passes,
        }
    }
}

// --- Hive results ---

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

impl std::fmt::Display for HiveResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}/{} ({:.02}%)",
            self.display_name, self.passed_tests, self.total_tests, self.success_percentage
        )
    }
}

/// Tracks excluded test counts per exclusion rule.
struct ExcludedCounts {
    passed: usize,
    total: usize,
}

fn calculate_success_percentage(passed_tests: usize, total_tests: usize) -> f64 {
    if total_tests == 0 {
        0.0
    } else {
        (passed_tests as f64 / total_tests as f64) * 100.0
    }
}

/// Maps a hive suite name to (category, display_name).
fn suite_to_category(suite: &str, fork: &str) -> (String, String) {
    let (category, display_name) = match suite {
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
        "eels/consume-rlp" => ("EVM - Consume RLP", fork),
        "eels/consume-engine" => ("EVM - Consume Engine", fork),
        "eels/execute-blobs" => ("EVM - Execute Blobs", "Execute Blobs"),
        other => {
            eprintln!("Warn: Unknown suite: {other}. Skipping");
            ("", "")
        }
    };
    (category.to_string(), display_name.to_string())
}

/// Process test cases, partitioning them into main results and excluded buckets.
fn process_tests<'a>(
    test_cases: impl Iterator<Item = &'a TestCase>,
    category: &str,
    display_name: &str,
    config: &ExclusionConfig,
    excluded: &mut HashMap<(ExclusionKind, usize), ExcludedCounts>,
) -> (usize, usize) {
    let mut main_passed = 0usize;
    let mut main_total = 0usize;

    for tc in test_cases {
        let passes = tc.summary_result.pass;
        if let Some((kind, idx)) = config.classify(category, display_name, &tc.name, passes) {
            let entry = excluded
                .entry((kind, idx))
                .or_insert(ExcludedCounts { passed: 0, total: 0 });
            entry.total += 1;
            if passes {
                entry.passed += 1;
            }
        } else {
            main_total += 1;
            if passes {
                main_passed += 1;
            }
        }
    }

    (main_passed, main_total)
}

fn fork_order(display_name: &str) -> Option<u8> {
    match display_name {
        "Amsterdam" => Some(0),
        "Osaka" => Some(1),
        "Prague" => Some(2),
        "Cancun" => Some(3),
        "Shanghai" => Some(4),
        "Paris" => Some(5),
        _ => None,
    }
}

fn sort_results(results: &mut [HiveResult]) {
    results.sort_by(|a, b| {
        let category_cmp = a.category.cmp(&b.category);
        if category_cmp != Ordering::Equal {
            return category_cmp;
        }

        if let (Some(rank_a), Some(rank_b)) =
            (fork_order(&a.display_name), fork_order(&b.display_name))
        {
            let order_cmp = rank_a.cmp(&rank_b);
            if order_cmp != Ordering::Equal {
                return order_cmp;
            }
        }

        b.passed_tests
            .cmp(&a.passed_tests)
            .then_with(|| {
                b.success_percentage
                    .partial_cmp(&a.success_percentage)
                    .unwrap()
            })
    });
}

fn group_results(results: Vec<HiveResult>) -> Vec<CategoryResults> {
    let mut grouped: Vec<CategoryResults> = Vec::new();
    for result in results {
        if let Some(last) = grouped
            .last_mut()
            .filter(|last| last.name == result.category)
        {
            last.tests.push(result);
            continue;
        }
        let name = result.category.clone();
        grouped.push(CategoryResults {
            name,
            tests: vec![result],
        });
    }
    grouped
}

fn build_slack_blocks(
    categories: &[CategoryResults],
    total_passed: usize,
    total_tests: usize,
    excluded_sections: &[(ExclusionKind, &ExclusionRule, &ExcludedCounts)],
) -> serde_json::Value {
    let total_percentage = calculate_success_percentage(total_passed, total_tests);
    let mut blocks = vec![json!({
        "type": "header",
        "text": {
            "type": "plain_text",
            "text": format!(
                "Daily Hive Coverage report \u{2014} {total_passed}/{total_tests} ({total_percentage:.02}%)"
            )
        }
    })];

    for category in categories {
        let category_passed = category.total_passed();
        let category_total = category.total_tests();
        let category_percentage = category.success_percentage();
        let status = if category_passed == category_total {
            "\u{2705}"
        } else {
            "\u{26a0}\u{fe0f}"
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

    // Add exclusion sections if there are any excluded tests
    let known_issues: Vec<_> = excluded_sections
        .iter()
        .filter(|(kind, _, counts)| *kind == ExclusionKind::KnownIssue && counts.total > 0)
        .collect();

    let wip: Vec<_> = excluded_sections
        .iter()
        .filter(|(kind, _, counts)| *kind == ExclusionKind::Wip && counts.total > 0)
        .collect();

    if !known_issues.is_empty() || !wip.is_empty() {
        blocks.push(json!({
            "type": "divider",
        }));
    }

    if !known_issues.is_empty() {
        let total_excluded: usize = known_issues.iter().map(|(_, _, c)| c.total).sum();
        let mut lines = vec![format!(
            "\u{1f4cb} *Known Issues* ({total_excluded} tests excluded)"
        )];
        for (_, rule, counts) in &known_issues {
            let failing = counts.total - counts.passed;
            lines.push(format!(
                "  {} > {}: {}/{} ({} failing)",
                rule.category, rule.subcategory, counts.passed, counts.total, failing
            ));
            lines.push(format!("  _{}_", rule.reason));
        }
        blocks.push(json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": lines.join("\n"),
            },
        }));
    }

    if !wip.is_empty() {
        let total_excluded: usize = wip.iter().map(|(_, _, c)| c.total).sum();
        let mut lines = vec![format!(
            "\u{1f6a7} *Work in Progress* ({total_excluded} tests excluded)"
        )];
        for (_, rule, counts) in &wip {
            let failing = counts.total - counts.passed;
            lines.push(format!(
                "  {} > {}: {}/{} ({} failing)",
                rule.category, rule.subcategory, counts.passed, counts.total, failing
            ));
            lines.push(format!("  _{}_", rule.reason));
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
    category: String,
    display_name: String,
    passed_tests: usize,
    total_tests: usize,
) {
    if category.is_empty() {
        return;
    }
    let entry = aggregated_results
        .entry((category, display_name))
        .or_insert((0, 0));
    entry.0 += passed_tests;
    entry.1 += total_tests;
}

const FORK_PATTERNS: &[(&str, &str)] = &[
    ("Paris", "fork_Paris"),
    ("Shanghai", "fork_Shanghai"),
    ("Cancun", "fork_Cancun"),
    ("Prague", "fork_Prague"),
    ("Osaka", "fork_Osaka"),
    ("Amsterdam", "fork_Amsterdam"),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ExclusionConfig::load();
    let mut aggregated_results: HashMap<(String, String), (usize, usize)> = HashMap::new();
    let mut excluded_counts: HashMap<(ExclusionKind, usize), ExcludedCounts> = HashMap::new();

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

            if json_data.name.as_str() == "eels/consume-rlp"
                || json_data.name.as_str() == "eels/consume-engine"
            {
                for (fork, pattern) in FORK_PATTERNS {
                    let (category, display_name) =
                        suite_to_category(&json_data.name, fork);
                    if category.is_empty() {
                        continue;
                    }

                    let fork_tests = json_data
                        .test_cases
                        .values()
                        .filter(|tc| tc.name.contains(pattern));

                    let (passed, total) = process_tests(
                        fork_tests,
                        &category,
                        &display_name,
                        &config,
                        &mut excluded_counts,
                    );

                    aggregate_result(
                        &mut aggregated_results,
                        category,
                        display_name,
                        passed,
                        total,
                    );
                }
            } else {
                let (category, display_name) =
                    suite_to_category(&json_data.name, "");
                if category.is_empty() {
                    continue;
                }

                let (passed, total) = process_tests(
                    json_data.test_cases.values(),
                    &category,
                    &display_name,
                    &config,
                    &mut excluded_counts,
                );

                aggregate_result(
                    &mut aggregated_results,
                    category,
                    display_name,
                    passed,
                    total,
                );
            }
        }
    }

    let mut results: Vec<HiveResult> = aggregated_results
        .into_iter()
        .filter(|(_, (_, total))| *total > 0)
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

    sort_results(&mut results);
    let grouped_results = group_results(results);

    for category in &grouped_results {
        println!("*{}*", category.name);
        for result in &category.tests {
            println!("\t{result}");
        }
        println!();
    }

    println!();
    let total_passed: usize = grouped_results
        .iter()
        .flat_map(|group| group.tests.iter().map(|r| r.passed_tests))
        .sum();
    let total_tests: usize = grouped_results
        .iter()
        .flat_map(|group| group.tests.iter().map(|r| r.total_tests))
        .sum();
    let total_percentage = calculate_success_percentage(total_passed, total_tests);
    println!("*Total: {total_passed}/{total_tests} ({total_percentage:.02}%)*");

    // Print exclusion summary to stdout
    for ((kind, idx), counts) in &excluded_counts {
        if counts.total == 0 {
            continue;
        }
        let rule = config.rule(*kind, *idx);
        let label = match kind {
            ExclusionKind::KnownIssue => "Known Issue",
            ExclusionKind::Wip => "WIP",
        };
        let failing = counts.total - counts.passed;
        println!(
            "\n[{label}] {} > {}: {}/{} ({failing} failing) — {}",
            rule.category, rule.subcategory, counts.passed, counts.total, rule.reason
        );
    }

    // Build excluded sections list for Slack, sorted for stable output
    let mut excluded_sections: Vec<(ExclusionKind, &ExclusionRule, &ExcludedCounts)> =
        excluded_counts
            .iter()
            .map(|((kind, idx), counts)| (*kind, config.rule(*kind, *idx), counts))
            .filter(|(_, _, counts)| counts.total > 0)
            .collect();
    excluded_sections.sort_by(|a, b| {
        a.1.category
            .cmp(&b.1.category)
            .then_with(|| a.1.subcategory.cmp(&b.1.subcategory))
    });

    let slack_blocks =
        build_slack_blocks(&grouped_results, total_passed, total_tests, &excluded_sections);
    fs::write(
        HIVE_SLACK_BLOCKS_FILE_PATH,
        serde_json::to_string_pretty(&slack_blocks)?,
    )?;

    Ok(())
}
