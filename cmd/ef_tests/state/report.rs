use crate::runner::{EFTestRunnerError, InternalError};
use colored::Colorize;
use ethrex_common::{
    Address, H256,
    types::{Account, AccountUpdate, Fork},
};
use ethrex_levm::errors::{ExecutionReport, TxResult, VMError};
use ethrex_vm::EvmError;
use itertools::Itertools;
use revm::primitives::{EVMError as RevmError, ExecutionResult as RevmExecutionResult};
use serde::{Deserialize, Serialize};
use spinoff::{Color, Spinner, spinners::Dots};
use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Display},
    path::PathBuf,
    time::Duration,
};

pub const LEVM_EF_TESTS_SUMMARY_SLACK_FILE_PATH: &str = "./levm_ef_tests_summary_slack.txt";
pub const LEVM_EF_TESTS_SUMMARY_GITHUB_FILE_PATH: &str = "./levm_ef_tests_summary_github.txt";
pub const EF_TESTS_CACHE_FILE_PATH: &str = "./levm_ef_tests_cache.json";

pub type TestVector = (usize, usize, usize);

pub fn progress(reports: &[EFTestReport], time: Duration) -> String {
    format!(
        "{}: {} {} {} - {}",
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

pub fn cache(reports: &[EFTestReport]) -> Result<PathBuf, EFTestRunnerError> {
    let cache_file_path = PathBuf::from(EF_TESTS_CACHE_FILE_PATH);
    let cache = serde_json::to_string_pretty(&reports).map_err(|err| {
        EFTestRunnerError::Internal(InternalError::MainRunnerInternal(format!(
            "Failed to serialize cache: {err}"
        )))
    })?;
    std::fs::write(&cache_file_path, cache).map_err(|err| {
        EFTestRunnerError::Internal(InternalError::MainRunnerInternal(format!(
            "Failed to write cache to file: {err}"
        )))
    })?;
    Ok(cache_file_path)
}

pub fn load() -> Result<Vec<EFTestReport>, EFTestRunnerError> {
    let mut reports_loading_spinner =
        Spinner::new(Dots, "Loading reports...".to_owned(), Color::Cyan);
    match std::fs::read_to_string(EF_TESTS_CACHE_FILE_PATH).ok() {
        Some(cache) => {
            reports_loading_spinner.success("Reports loaded");
            serde_json::from_str(&cache).map_err(|err| {
                EFTestRunnerError::Internal(InternalError::MainRunnerInternal(format!(
                    "Cache exists but there was an error loading it: {err}"
                )))
            })
        }
        None => {
            reports_loading_spinner.success("No cache found");
            Ok(Vec::default())
        }
    }
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
        "{} {}/{total_run} ({success_percentage:.2}%)\n\n{}\n{}\n{}\n{}\n\n\n{}\n",
        "Summary:".bold(),
        if total_passed == total_run {
            format!("{}", total_passed).green()
        } else if total_passed > 0 {
            format!("{}", total_passed).yellow()
        } else {
            format!("{}", total_passed).red()
        },
        // NOTE: Keep in order, see the Fork Enum to check
        // NOTE: Uncomment the summaries if EF tests for those specific forks exist.

        // fork_summary_shell(reports, Fork::Osaka),
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
            format!("{}", fork_passed_tests).green()
        } else if fork_passed_tests > 0 {
            format!("{}", fork_passed_tests).yellow()
        } else {
            format!("{}", fork_passed_tests).red()
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
                "{}: {}/{} ({:.2}%)\n",
                dir.bold(),
                if total_passed == total_run {
                    format!("{}", total_passed).green()
                } else if total_passed > 0 {
                    format!("{}", total_passed).yellow()
                } else {
                    format!("{}", total_passed).red()
                },
                total_run,
                success_percentage
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
            writeln!(f, "Test: {}", report.name)?;
            writeln!(f)?;
            for (fork, result) in &report.fork_results {
                if result.failed_vectors.is_empty() {
                    continue;
                }
                writeln!(f, "\tFork: {:?}", fork)?;
                for (failed_vector, error) in &result.failed_vectors {
                    writeln!(
                        f,
                        "\t\tFailed Vector: (data_index: {}, gas_limit_index: {}, value_index: {})",
                        failed_vector.0, failed_vector.1, failed_vector.2
                    )?;
                    writeln!(f, "\t\t\tError: {error}")?;
                    if let Some(re_run_report) = &report.re_run_report {
                        if let Some(execution_report) =
                            re_run_report.execution_report.get(&(*failed_vector, *fork))
                        {
                            if let Some((levm_result, revm_result)) =
                                &execution_report.execution_result_mismatch
                            {
                                writeln!(
                                    f,
                                    "\t\t\tExecution result mismatch: LEVM: {levm_result:?}, REVM: {revm_result:?}",
                                )?;
                            }
                            if let Some((levm_gas_used, revm_gas_used)) =
                                &execution_report.gas_used_mismatch
                            {
                                writeln!(
                                    f,
                                    "\t\t\tGas used mismatch: LEVM: {levm_gas_used}, REVM: {revm_gas_used} (diff: {})",
                                    levm_gas_used.abs_diff(*revm_gas_used)
                                )?;
                            }
                            if let Some((levm_gas_refunded, revm_gas_refunded)) =
                                &execution_report.gas_refunded_mismatch
                            {
                                writeln!(
                                    f,
                                    "\t\t\tGas refunded mismatch: LEVM: {levm_gas_refunded}, REVM: {revm_gas_refunded} (diff: {})",
                                    levm_gas_refunded.abs_diff(*revm_gas_refunded)
                                )?;
                            }
                            if let Some((levm_logs, revm_logs)) = &execution_report.logs_mismatch {
                                writeln!(f, "\t\t\tLogs mismatch:")?;
                                writeln!(f, "\t\t\t\tLevm Logs: ")?;
                                let levm_log_report = levm_logs.iter().map(|log| format!(
                                            "\t\t\t\t Log {{ address: {:#x}, topic: {:?}, data: {:#x} }} \n",
                                            log.address, log.topics, log.data
                                        ))
                                        .fold(String::new(), |acc, arg| acc + arg.as_str());
                                writeln!(f, "{}", levm_log_report)?;
                                writeln!(f, "\t\t\t\tRevm Logs: ")?;
                                let revm_log_report = revm_logs
                                    .iter()
                                    .map(|log| format!("\t\t\t\t {:?} \n", log))
                                    .fold(String::new(), |acc, arg| acc + arg.as_str());
                                writeln!(f, "{}", revm_log_report)?;
                            }
                            if let Some((levm_result, revm_error)) =
                                &execution_report.re_runner_error
                            {
                                writeln!(
                                    f,
                                    "\t\t\tRe-run error: LEVM: {levm_result:?}, REVM: {revm_error}",
                                )?;
                            }
                        }

                        if let Some(account_update) = re_run_report
                            .account_updates_report
                            .get(&(*failed_vector, *fork))
                        {
                            writeln!(f, "\t\t\t{}", &account_update.to_string())?;
                        } else {
                            writeln!(
                                f,
                                "\t\t\tNo account updates report found. Account update reports are only generated for tests that failed at the post-state validation stage."
                            )?;
                        }
                    } else {
                        writeln!(
                            f,
                            "\t\t\tNo re-run report found. Re-run reports are only generated for tests that failed at the post-state validation stage."
                        )?;
                    }
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
    pub test_hash: H256,
    pub re_run_report: Option<TestReRunReport>,
    pub fork_results: HashMap<Fork, EFTestReportForkResult>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EFTestReportForkResult {
    pub skipped: bool,
    pub failed_vectors: HashMap<TestVector, EFTestRunnerError>,
}

impl EFTestReport {
    pub fn new(name: String, dir: String, test_hash: H256) -> Self {
        EFTestReport {
            name,
            dir,
            test_hash,
            re_run_report: None,
            fork_results: HashMap::new(),
        }
    }

    pub fn register_re_run_report(&mut self, re_run_report: TestReRunReport) {
        self.re_run_report = Some(re_run_report);
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
        levm_cache: HashMap<Address, Account>,
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub expected_post_state_root: H256,
    pub levm_post_state_root: H256,
    pub revm_post_state_root: H256,
    pub initial_accounts: HashMap<Address, Account>,
    pub levm_account_updates: Vec<AccountUpdate>,
    pub revm_account_updates: Vec<AccountUpdate>,
    pub levm_updated_accounts_only: HashSet<Address>,
    pub revm_updated_accounts_only: HashSet<Address>,
    pub shared_updated_accounts: HashSet<Address>,
}

impl fmt::Display for ComparisonReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.revm_post_state_root != self.expected_post_state_root {
            writeln!(f, "\n\t\t\tWARNING: REVM fails this test")?;
            if self.levm_post_state_root != self.revm_post_state_root {
                writeln!(f, "\t\t\tPost-state root mismatch between LEVM and REVM\n")?;
            } else {
                writeln!(f, "\t\t\tSame Post-state root in LEVM and REVM\n")?;
            }
        } else {
            writeln!(f, "\n\t\t\tREVM passes this test")?;
        }

        let all_updated_accounts = &(&self.levm_updated_accounts_only
            | &self.revm_updated_accounts_only)
            | &self.shared_updated_accounts;

        for address in all_updated_accounts {
            writeln!(f, "\n\t\t\t{address:#x}:")?;

            let account_updates_for_address: Vec<(String, AccountUpdate)> =
                if self.levm_updated_accounts_only.contains(&address) {
                    writeln!(f, "\t\t\t\tWas updated in LEVM but not in REVM")?;
                    self.levm_account_updates
                        .clone()
                        .iter()
                        .filter(|account_update| account_update.address == address)
                        .map(|account_update| ("LEVM".to_string(), account_update.clone()))
                        .collect()
                } else if self.revm_updated_accounts_only.contains(&address) {
                    writeln!(f, "\t\t\t\tWas updated in REVM but not in LEVM")?;
                    self.revm_account_updates
                        .clone()
                        .iter()
                        .filter(|account_update| account_update.address == address)
                        .map(|account_update| ("REVM".to_string(), account_update.clone()))
                        .collect()
                } else {
                    writeln!(f, "\t\t\t\tWas updated in both LEVM and REVM")?;
                    [
                        self.revm_account_updates
                            .clone()
                            .iter()
                            .filter(|account_update| account_update.address == address)
                            .map(|account_update| ("REVM".to_string(), account_update.clone()))
                            .collect::<Vec<_>>(),
                        self.levm_account_updates
                            .clone()
                            .iter()
                            .filter(|account_update| account_update.address == address)
                            .map(|account_update| ("LEVM".to_string(), account_update.clone()))
                            .collect::<Vec<_>>(),
                    ]
                    .concat()
                };

            // Account before Tx execution
            let base_account = self
                .initial_accounts
                .get(&address)
                .cloned()
                .unwrap_or_default();

            for (vm, account_update) in &account_updates_for_address {
                writeln!(f, "\t\t\t\t{vm} Account Update:")?;

                if account_update.removed {
                    writeln!(f, "\t\t\t\t\tAccount was removed")?;
                    continue;
                }

                // Display changes in Account Info
                if let Some(new_info) = &account_update.info {
                    writeln!(
                        f,
                        "\t\t\t\t\tNonce: {} -> {}",
                        base_account.info.nonce, new_info.nonce
                    )?;
                    writeln!(
                        f,
                        "\t\t\t\t\tBalance: {} -> {}",
                        base_account.info.balance, new_info.balance
                    )?;

                    if base_account.info.code_hash != new_info.code_hash {
                        writeln!(
                            f,
                            "\t\t\t\t\tCode: {} -> {}",
                            if base_account.code.is_empty() {
                                "empty".to_string()
                            } else {
                                hex::encode(&base_account.code)
                            },
                            account_update
                                .code
                                .as_ref()
                                .map(|code| if code.is_empty() {
                                    "empty".to_string()
                                } else {
                                    hex::encode(code)
                                })
                                .expect("If code hash changed then 'code' shouldn't be None.")
                        )?;
                    }
                }

                for (key, value) in &account_update.added_storage {
                    let initial_value = base_account.storage.get(key).cloned().unwrap_or_default();
                    writeln!(
                        f,
                        "\t\t\t\t\tStorage slot: {key:#x}: {} -> {}",
                        initial_value, value
                    )?;
                }
            }

            if self.shared_updated_accounts.contains(&address) {
                let levm_account_update = account_updates_for_address
                    .iter()
                    .find(|(vm, _)| vm == "LEVM")
                    .map(|(_, update)| update)
                    .expect("LEVM account update not found");
                let revm_account_update = account_updates_for_address
                    .iter()
                    .find(|(vm, _)| vm == "REVM")
                    .map(|(_, update)| update)
                    .expect("REVM account update not found");

                if levm_account_update == revm_account_update {
                    writeln!(f, "\t\t\t\tNo differences between updates")?;
                    continue;
                }

                if levm_account_update.removed != revm_account_update.removed {
                    writeln!(
                        f,
                        "\t\t\t\tAccount removal mismatch: LEVM: {}, REVM: {}",
                        levm_account_update.removed, revm_account_update.removed
                    )?;
                }

                if levm_account_update.info != revm_account_update.info {
                    match (&levm_account_update.info, &revm_account_update.info) {
                        (Some(levm_info), Some(revm_info)) => {
                            if levm_info.nonce != revm_info.nonce {
                                writeln!(
                                    f,
                                    "\t\t\t\tNonce mismatch: LEVM: {}, REVM: {}",
                                    levm_info.nonce, revm_info.nonce
                                )?;
                            }
                            if levm_info.balance != revm_info.balance {
                                writeln!(
                                    f,
                                    "\t\t\t\tBalance mismatch: LEVM: {}, REVM: {}",
                                    levm_info.balance, revm_info.balance
                                )?;
                            }
                        }
                        (Some(levm_info), None) => {
                            writeln!(
                                f,
                                "\t\t\t\tLEVM has account info but REVM does not: Nonce: {}, Balance: {}",
                                levm_info.nonce, levm_info.balance
                            )?;
                        }
                        (None, Some(revm_info)) => {
                            writeln!(
                                f,
                                "\t\t\t\tREVM has account info but LEVM does not: Nonce: {}, Balance: {}",
                                revm_info.nonce, revm_info.balance
                            )?;
                        }
                        (None, None) => {
                            // No account info in either LEVM or REVM, nothing to report.
                        }
                    }
                }

                // Compare all storage changes between LEVM and REVM.
                let all_keys: HashSet<_> = levm_account_update
                    .added_storage
                    .keys()
                    .chain(revm_account_update.added_storage.keys())
                    .collect();

                for key in all_keys {
                    let levm_value = levm_account_update
                        .added_storage
                        .get(key)
                        .cloned()
                        .unwrap_or_default();
                    let revm_value = revm_account_update
                        .added_storage
                        .get(key)
                        .cloned()
                        .unwrap_or_default();

                    if levm_value != revm_value {
                        writeln!(
                            f,
                            "\t\t\t\tStorage slot mismatch at key {key:#x}: LEVM: {}, REVM: {}",
                            levm_value, revm_value
                        )?;
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TestReRunExecutionReport {
    pub execution_result_mismatch: Option<(TxResult, RevmExecutionResult)>,
    pub gas_used_mismatch: Option<(u64, u64)>,
    pub gas_refunded_mismatch: Option<(u64, u64)>,
    pub logs_mismatch: Option<(Vec<ethrex_common::types::Log>, Vec<revm::primitives::Log>)>,
    pub re_runner_error: Option<(TxResult, String)>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TestReRunReport {
    pub execution_report: HashMap<(TestVector, Fork), TestReRunExecutionReport>,
    pub account_updates_report: HashMap<(TestVector, Fork), ComparisonReport>,
}

impl TestReRunReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_execution_result_mismatch(
        &mut self,
        vector: TestVector,
        levm_result: TxResult,
        revm_result: RevmExecutionResult,
        fork: Fork,
    ) {
        let value = Some((levm_result, revm_result));
        self.execution_report
            .entry((vector, fork))
            .and_modify(|report| {
                report.execution_result_mismatch = value.clone();
            })
            .or_insert(TestReRunExecutionReport {
                execution_result_mismatch: value,
                ..Default::default()
            });
    }

    pub fn register_gas_used_mismatch(
        &mut self,
        vector: TestVector,
        levm_gas_used: u64,
        revm_gas_used: u64,
        fork: Fork,
    ) {
        let value = Some((levm_gas_used, revm_gas_used));
        self.execution_report
            .entry((vector, fork))
            .and_modify(|report| {
                report.gas_used_mismatch = value;
            })
            .or_insert(TestReRunExecutionReport {
                gas_used_mismatch: value,
                ..Default::default()
            });
    }

    pub fn register_gas_refunded_mismatch(
        &mut self,
        vector: TestVector,
        levm_gas_refunded: u64,
        revm_gas_refunded: u64,
        fork: Fork,
    ) {
        let value = Some((levm_gas_refunded, revm_gas_refunded));
        self.execution_report
            .entry((vector, fork))
            .and_modify(|report| {
                report.gas_refunded_mismatch = value;
            })
            .or_insert(TestReRunExecutionReport {
                gas_refunded_mismatch: value,
                ..Default::default()
            });
    }

    pub fn register_logs_mismatch(
        &mut self,
        vector: TestVector,
        levm_logs: Vec<ethrex_common::types::Log>,
        revm_logs: Vec<revm::primitives::Log>,
        fork: Fork,
    ) {
        let value = Some((levm_logs, revm_logs));
        self.execution_report
            .entry((vector, fork))
            .and_modify(|report| {
                report.logs_mismatch = value.clone();
            })
            .or_insert(TestReRunExecutionReport {
                logs_mismatch: value,
                ..Default::default()
            });
    }

    pub fn register_account_updates_report(
        &mut self,
        vector: TestVector,
        report: ComparisonReport,
        fork: Fork,
    ) {
        self.account_updates_report.insert((vector, fork), report);
    }

    pub fn register_re_run_failure(
        &mut self,
        vector: TestVector,
        levm_result: TxResult,
        revm_error: RevmError<EvmError>,
        fork: Fork,
    ) {
        let value = Some((levm_result, revm_error.to_string()));
        self.execution_report
            .entry((vector, fork))
            .and_modify(|report| {
                report.re_runner_error = value.clone();
            })
            .or_insert(TestReRunExecutionReport {
                re_runner_error: value,
                ..Default::default()
            });
    }
}
