use crate::parser::get_test_relative_path;
use crate::runner::{EFTestRunnerError, InternalError};
use crate::types::EFTestInfo;
use colored::Colorize;
use ethrex_common::{Address, H256, types::Fork};
use ethrex_levm::account::LevmAccount;
use ethrex_levm::errors::{ExecutionReport, VMError};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::{self, Display},
    path::PathBuf,
    time::Duration,
};

pub const LEVM_EF_TESTS_SUMMARY_SLACK_FILE_PATH: &str = "./levm_ef_tests_summary_slack.txt";
pub const LEVM_EF_TESTS_SUMMARY_GITHUB_FILE_PATH: &str = "./levm_ef_tests_summary_github.txt";

pub type TestVector = (usize, usize, usize);

pub fn progress(reports: &[EFTestReport], time: Duration) -> String {
    format!(
        "\r{}: {} {} {} - {}",
        "Ethereum Foundation Tests".bold(),
        format!(
            "{} passed",
            reports.iter().filter(|report| report.passed()).count()
        )
        .green()
        .bold(),
        format!(
            "{} failed",
            reports.iter().filter(|report| !report.passed()).count()
        )
        .red()
        .bold(),
        format!("{} total run", reports.len()).blue().bold(),
        format_duration_as_mm_ss(time)
    )
}

pub fn format_duration_as_mm_ss(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

pub fn write(reports: &[EFTestReport]) -> Result<PathBuf, EFTestRunnerError> {
    let report_file_path = PathBuf::from("./levm_ef_tests_report.txt");
    let failed_test_reports = EFTestsReport(
        reports
            .iter()
            .filter(|&report| !report.passed())
            .cloned()
            .collect(),
    );
    std::fs::write(
        "./levm_ef_tests_report.txt",
        failed_test_reports.to_string(),
    )
    .map_err(|err| {
        EFTestRunnerError::Internal(InternalError::MainRunnerInternal(format!(
            "Failed to write report to file: {err}"
        )))
    })?;
    Ok(report_file_path)
}

pub fn summary_for_slack(reports: &[EFTestReport]) -> String {
    let total_passed = total_fork_test_passed(reports);
    let total_run = total_fork_test_run(reports);
    let success_percentage = (total_passed as f64 / total_run as f64) * 100.0;
    format!(
        r#"{{
    "blocks": [
        {{
            "type": "header",
            "text": {{
                "type": "plain_text",
                "text": "Daily LEVM EF Tests Run Report"
            }}
        }},
        {{
            "type": "divider"
        }},
        {{
            "type": "section",
            "text": {{
                "type": "mrkdwn",
                "text": "*Summary*: {total_passed}/{total_run} ({success_percentage:.2}%)\n\n{}\n{}\n{}\n{}\n"
            }}
        }}
    ]
}}"#,
        fork_summary_for_slack(reports, Fork::Prague),
        fork_summary_for_slack(reports, Fork::Cancun),
        fork_summary_for_slack(reports, Fork::Shanghai),
        fork_summary_for_slack(reports, Fork::Paris),
    )
}

fn fork_summary_for_slack(reports: &[EFTestReport], fork: Fork) -> String {
    let fork_str: &str = fork.into();
    let (fork_tests, fork_passed_tests, fork_success_percentage) = fork_statistics(reports, fork);
    format!(r#"*{fork_str}:* {fork_passed_tests}/{fork_tests} ({fork_success_percentage:.2}%)"#)
}

pub fn write_summary_for_slack(reports: &[EFTestReport]) -> Result<PathBuf, EFTestRunnerError> {
    let summary_file_path = PathBuf::from(LEVM_EF_TESTS_SUMMARY_SLACK_FILE_PATH);
    std::fs::write(
        LEVM_EF_TESTS_SUMMARY_SLACK_FILE_PATH,
        summary_for_slack(reports),
    )
    .map_err(|err| {
        EFTestRunnerError::Internal(InternalError::MainRunnerInternal(format!(
            "Failed to write summary to file: {err}"
        )))
    })?;
    Ok(summary_file_path)
}

pub fn summary_for_github(reports: &[EFTestReport]) -> String {
    let total_passed = total_fork_test_passed(reports);
    let total_run = total_fork_test_run(reports);
    let success_percentage = (total_passed as f64 / total_run as f64) * 100.0;
    format!(
        r#"Summary: {total_passed}/{total_run} ({success_percentage:.2}%)\n\n{}\n{}\n{}\n{}\n"#,
        fork_summary_for_github(reports, Fork::Prague),
        fork_summary_for_github(reports, Fork::Cancun),
        fork_summary_for_github(reports, Fork::Shanghai),
        fork_summary_for_github(reports, Fork::Paris),
    )
}

fn fork_summary_for_github(reports: &[EFTestReport], fork: Fork) -> String {
    let fork_str: &str = fork.into();
    let (fork_tests, fork_passed_tests, fork_success_percentage) = fork_statistics(reports, fork);
    format!("{fork_str}: {fork_passed_tests}/{fork_tests} ({fork_success_percentage:.2}%)")
}

pub fn write_summary_for_github(reports: &[EFTestReport]) -> Result<PathBuf, EFTestRunnerError> {
    let summary_file_path = PathBuf::from(LEVM_EF_TESTS_SUMMARY_GITHUB_FILE_PATH);
    std::fs::write(
        LEVM_EF_TESTS_SUMMARY_GITHUB_FILE_PATH,
        summary_for_github(reports),
    )
    .map_err(|err| {
        EFTestRunnerError::Internal(InternalError::MainRunnerInternal(format!(
            "Failed to write summary to file: {err}"
        )))
    })?;
    Ok(summary_file_path)
}

pub fn summary_for_shell(reports: &[EFTestReport]) -> String {
    let total_passed = total_fork_test_passed(reports);
    let total_run = total_fork_test_run(reports);
    let success_percentage = (total_passed as f64 / total_run as f64) * 100.0;
    format!(
        "{} {}/{total_run} ({success_percentage:.2}%)\n\n{}\n{}\n{}\n{}\n{}\n\n\n{}\n",
        "Summary:".bold(),
        if total_passed == total_run {
            format!("{total_passed}").green()
        } else if total_passed > 0 {
            format!("{total_passed}").yellow()
        } else {
            format!("{total_passed}").red()
        },
        // NOTE: Keep in order, see the Fork Enum to check
        // NOTE: Uncomment the summaries if EF tests for those specific forks exist.
        fork_summary_shell(reports, Fork::Osaka),
        fork_summary_shell(reports, Fork::Prague),
        fork_summary_shell(reports, Fork::Cancun),
        fork_summary_shell(reports, Fork::Shanghai),
        fork_summary_shell(reports, Fork::Paris),
        test_dir_summary_for_shell(reports),
    )
}

fn fork_summary_shell(reports: &[EFTestReport], fork: Fork) -> String {
    let fork_str: &str = fork.into();
    let (fork_tests, fork_passed_tests, fork_success_percentage) = fork_statistics(reports, fork);
    format!(
        "{}: {}/{fork_tests} ({fork_success_percentage:.2}%)",
        fork_str.bold(),
        if fork_passed_tests == fork_tests {
            format!("{fork_passed_tests}").green()
        } else if fork_passed_tests > 0 {
            format!("{fork_passed_tests}").yellow()
        } else {
            format!("{fork_passed_tests}").red()
        },
    )
}

fn fork_statistics(reports: &[EFTestReport], fork: Fork) -> (usize, usize, f64) {
    let fork_tests = reports
        .iter()
        .filter(|report| report.fork_results.contains_key(&fork))
        .count();
    let fork_passed_tests = reports
        .iter()
        .filter(|report| match report.fork_results.get(&fork) {
            Some(result) => result.failed_vectors.is_empty(),
            None => false,
        })
        .count();
    let fork_success_percentage = (fork_passed_tests as f64 / fork_tests as f64) * 100.0;
    (fork_tests, fork_passed_tests, fork_success_percentage)
}

pub fn test_dir_summary_for_shell(reports: &[EFTestReport]) -> String {
    let mut test_dirs_summary = String::new();
    reports
        .iter()
        .into_group_map_by(|report| report.dir.clone())
        .iter()
        .map(|(dir, reports)| {
            let total_passed =
                total_fork_test_passed(&reports.iter().map(|&r| r.clone()).collect::<Vec<_>>());
            let total_run =
                total_fork_test_run(&reports.iter().map(|&r| r.clone()).collect::<Vec<_>>());
            if total_passed == 0 {
                (dir, reports, 0)
            } else if total_passed > 0 && total_passed < total_run {
                (dir, reports, 1)
            } else {
                (dir, reports, 2)
            }
        })
        .sorted_by_key(|(_dir, _reports, weight)| *weight)
        .rev()
        .for_each(|(dir, reports, _weight)| {
            let total_passed =
                total_fork_test_passed(&reports.iter().map(|&r| r.clone()).collect::<Vec<_>>());
            let total_run =
                total_fork_test_run(&reports.iter().map(|&r| r.clone()).collect::<Vec<_>>());
            let success_percentage = (total_passed as f64 / total_run as f64) * 100.0;
            let test_dir_summary = format!(
                "{}: {}/{total_run} ({success_percentage:.2}%)\n",
                get_test_relative_path(PathBuf::from(dir)).bold(),
                if total_passed == total_run {
                    format!("{total_passed}").green()
                } else if total_passed > 0 {
                    format!("{total_passed}").yellow()
                } else {
                    format!("{total_passed}").red()
                },
            );
            test_dirs_summary.push_str(&test_dir_summary);
        });
    test_dirs_summary
}

#[derive(Debug, Default, Clone)]
pub struct EFTestsReport(pub Vec<EFTestReport>);

pub fn total_fork_test_passed(reports: &[EFTestReport]) -> usize {
    let mut tests_passed = 0;
    for report in reports {
        for fork_result in report.fork_results.values() {
            if fork_result.failed_vectors.is_empty() {
                tests_passed += 1;
            }
        }
    }
    tests_passed
}

pub fn total_fork_test_run(reports: &[EFTestReport]) -> usize {
    let mut tests_run = 0;
    for report in reports {
        tests_run += report.fork_results.len();
    }
    tests_run
}

impl Display for EFTestsReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total_passed = total_fork_test_passed(&self.0);
        let total_run = total_fork_test_run(&self.0);
        writeln!(f, "Summary: {total_passed}/{total_run}",)?;
        writeln!(f)?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Osaka))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Prague))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Cancun))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Shanghai))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Paris))?;
        writeln!(f)?;
        writeln!(f, "Passed tests:")?;
        writeln!(f)?;
        writeln!(f, "{}", test_dir_summary_for_shell(&self.0))?;
        writeln!(f)?;
        writeln!(f, "Failed tests:")?;
        writeln!(f)?;
        for report in self.0.iter() {
            if report.passed() {
                continue;
            }
            writeln!(f, "{} \n{}", "Test:".bold(), report)?;
            writeln!(f)?;
            for (fork, result) in &report.fork_results {
                if result.failed_vectors.is_empty() {
                    continue;
                }
                writeln!(f, "\tFork: {fork:?}")?;
                for (failed_vector, error) in &result.failed_vectors {
                    writeln!(
                        f,
                        "\t\tFailed Vector: (data_index: {}, gas_limit_index: {}, value_index: {})",
                        failed_vector.0, failed_vector.1, failed_vector.2
                    )?;
                    writeln!(f, "\t\t\tError: {error}")?;
                    writeln!(f)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EFTestReport {
    pub name: String,
    pub dir: String,
    pub description: String,
    pub url: String,
    pub reference_spec: String,
    pub test_hash: H256,
    pub fork_results: HashMap<Fork, EFTestReportForkResult>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EFTestReportForkResult {
    pub skipped: bool,
    pub failed_vectors: HashMap<TestVector, EFTestRunnerError>,
}

impl EFTestReport {
    pub fn new(name: String, dir: String, info: EFTestInfo, test_hash: H256) -> Self {
        EFTestReport {
            name,
            dir,
            description: info
                .description
                .unwrap_or("No description provided by this tests".to_string()),
            url: info
                .url
                .unwrap_or("No url provided by this tests".to_string()),
            reference_spec: info
                .reference_spec
                .unwrap_or("No reference spec provided by this tests".to_string()),
            test_hash,
            fork_results: HashMap::new(),
        }
    }

    pub fn register_fork_result(
        &mut self,
        fork: Fork,
        ef_test_report_fork: EFTestReportForkResult,
    ) {
        self.fork_results.insert(fork, ef_test_report_fork);
    }

    pub fn passed(&self) -> bool {
        self.fork_results
            .values()
            .all(|fork_result| fork_result.failed_vectors.is_empty())
    }
}

impl Display for EFTestReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut json_name = String::from(""); //In some cases there are more than one tests per file, so the name of the tests and the name of the file are different.
        if self.name.contains("::") {
            json_name = self.name.clone().split("::").collect::<Vec<&str>>()[1]
                .split("[")
                .collect::<Vec<&str>>()[0]
                .strip_prefix("test")
                .unwrap_or("")
                .to_owned()
                + ".json";
        }
        writeln!(f, "Test name: {}", self.name)?;
        writeln!(f, "Test path: {}", self.dir.clone() + &json_name)?;
        writeln!(f)?;
        writeln!(f, "Test description: {}", self.description)?;
        writeln!(f)?;
        writeln!(
            f,
            "Note: The following links may help when debugging `ef-tests`:"
        )?;
        writeln!(
            f,
            "- https://ethereum-tests.readthedocs.io/en/latest/test_types/gstate_tests.html#"
        )?;
        writeln!(f, "- Test reference spec: {}", self.reference_spec)?;
        writeln!(f, "- Test url: {}", self.url)?;
        Ok(())
    }
}

impl EFTestReportForkResult {
    pub fn new() -> Self {
        Self {
            skipped: false,
            failed_vectors: HashMap::new(),
        }
    }

    pub fn register_unexpected_execution_failure(
        &mut self,
        error: VMError,
        failed_vector: TestVector,
    ) {
        self.failed_vectors.insert(
            failed_vector,
            EFTestRunnerError::ExecutionFailedUnexpectedly(error),
        );
    }

    pub fn register_vm_initialization_failure(
        &mut self,
        reason: String,
        failed_vector: TestVector,
    ) {
        self.failed_vectors.insert(
            failed_vector,
            EFTestRunnerError::VMInitializationFailed(reason),
        );
    }

    pub fn register_pre_state_validation_failure(
        &mut self,
        reason: String,
        failed_vector: TestVector,
    ) {
        self.failed_vectors.insert(
            failed_vector,
            EFTestRunnerError::FailedToEnsurePreState(reason),
        );
    }

    pub fn register_post_state_validation_failure(
        &mut self,
        transaction_report: ExecutionReport,
        reason: String,
        failed_vector: TestVector,
        levm_cache: BTreeMap<Address, LevmAccount>,
    ) {
        self.failed_vectors.insert(
            failed_vector,
            EFTestRunnerError::FailedToEnsurePostState(
                Box::new(transaction_report),
                reason,
                levm_cache,
            ),
        );
    }

    pub fn register_post_state_validation_error_mismatch(
        &mut self,
        reason: String,
        failed_vector: TestVector,
    ) {
        self.failed_vectors.insert(
            failed_vector,
            EFTestRunnerError::ExpectedExceptionDoesNotMatchReceived(reason),
        );
    }

    pub fn register_error_on_reverting_levm_state(
        &mut self,
        reason: String,
        failed_vector: TestVector,
    ) {
        self.failed_vectors.insert(
            failed_vector,
            EFTestRunnerError::FailedToRevertLEVMState(reason),
        );
    }

    pub fn register_failed_vector(&mut self, vector: TestVector, error: EFTestRunnerError) {
        self.failed_vectors.insert(vector, error);
    }
}
