use crate::runner::{EFTestRunnerError, InternalError};
use colored::Colorize;
use ethrex_common::{
    types::{Account, Fork},
    Address, H256,
};
use ethrex_levm::{
    errors::{ExecutionReport, TxResult, VMError},
    StorageSlot,
};
use ethrex_storage::{error::StoreError, AccountUpdate};
use itertools::Itertools;
use revm::primitives::{EVMError, ExecutionResult as RevmExecutionResult};
use serde::{Deserialize, Serialize};
use spinoff::{spinners::Dots, Color, Spinner};
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
                "text": "*Summary*: {total_passed}/{total_run} ({success_percentage:.2}%)\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n"
            }}
        }}
    ]
}}"#,
        fork_summary_for_slack(reports, Fork::Prague),
        fork_summary_for_slack(reports, Fork::Cancun),
        fork_summary_for_slack(reports, Fork::Shanghai),
        fork_summary_for_slack(reports, Fork::Byzantium),
        fork_summary_for_slack(reports, Fork::Berlin),
        fork_summary_for_slack(reports, Fork::Constantinople),
        fork_summary_for_slack(reports, Fork::Petersburg),
        fork_summary_for_slack(reports, Fork::Paris),
        fork_summary_for_slack(reports, Fork::Homestead),
        fork_summary_for_slack(reports, Fork::Istanbul),
        fork_summary_for_slack(reports, Fork::London),
        fork_summary_for_slack(reports, Fork::Frontier),
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
        r#"Summary: {total_passed}/{total_run} ({success_percentage:.2}%)\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n"#,
        fork_summary_for_github(reports, Fork::Prague),
        fork_summary_for_github(reports, Fork::Cancun),
        fork_summary_for_github(reports, Fork::Shanghai),
        fork_summary_for_github(reports, Fork::Byzantium),
        fork_summary_for_github(reports, Fork::Berlin),
        fork_summary_for_github(reports, Fork::Constantinople),
        fork_summary_for_github(reports, Fork::Petersburg),
        fork_summary_for_github(reports, Fork::Paris),
        fork_summary_for_github(reports, Fork::Homestead),
        fork_summary_for_github(reports, Fork::Istanbul),
        fork_summary_for_github(reports, Fork::London),
        fork_summary_for_github(reports, Fork::Frontier),
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
        "{} {}/{total_run} ({success_percentage:.2}%)\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n\n{}\n",
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
        // fork_summary_shell(reports, Fork::GrayGlacier),
        // fork_summary_shell(reports, Fork::ArrowGlacier),
        fork_summary_shell(reports, Fork::London),
        fork_summary_shell(reports, Fork::Berlin),
        // fork_summary_shell(reports, Fork::MuirGlacier),
        fork_summary_shell(reports, Fork::Istanbul),
        fork_summary_shell(reports, Fork::Petersburg),
        fork_summary_shell(reports, Fork::Constantinople),
        fork_summary_shell(reports, Fork::Byzantium),
        fork_summary_shell(reports, Fork::SpuriousDragon),
        fork_summary_shell(reports, Fork::Tangerine),
        // fork_summary_shell(reports, Fork::DaoFork),
        fork_summary_shell(reports, Fork::Homestead),
        // fork_summary_shell(reports, Fork::FrontierThawing),
        fork_summary_shell(reports, Fork::Frontier),

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
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Byzantium))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Berlin))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Constantinople))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Petersburg))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Paris))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Homestead))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Istanbul))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::London))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Frontier))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::SpuriousDragon))?;
        writeln!(f, "{}", fork_summary_shell(&self.0, Fork::Tangerine))?;
        writeln!(f)?;
        writeln!(f, "Failed tests:")?;
        writeln!(f)?;
        writeln!(f, "{}", test_dir_summary_for_shell(&self.0))?;
        for report in self.0.iter() {
            if report.passed() {
                continue;
            }
            writeln!(f, "Test: {}", report.name)?;
            writeln!(f)?;
            for (fork, result) in &report.fork_results {
                writeln!(f, "\n  Fork: {:?}", fork)?;
                if result.failed_vectors.is_empty() {
                    continue;
                }
                writeln!(f, "    Failed Vectors:")?;
                for (failed_vector, error) in &result.failed_vectors {
                    writeln!(
                        f,
                        "Vector: (data_index: {}, gas_limit_index: {}, value_index: {})",
                        failed_vector.0, failed_vector.1, failed_vector.2
                    )?;
                    writeln!(f, "Error: {error}")?;
                    if let Some(re_run_report) = &report.re_run_report {
                        if let Some(execution_report) =
                            re_run_report.execution_report.get(&(*failed_vector, *fork))
                        {
                            if let Some((levm_result, revm_result)) =
                                &execution_report.execution_result_mismatch
                            {
                                writeln!(
                                    f,
                                    "Execution result mismatch: LEVM: {levm_result:?}, REVM: {revm_result:?}",
                                )?;
                            }
                            if let Some((levm_gas_used, revm_gas_used)) =
                                &execution_report.gas_used_mismatch
                            {
                                writeln!(
                                    f,
                                    "Gas used mismatch: LEVM: {levm_gas_used}, REVM: {revm_gas_used} (diff: {})",
                                    levm_gas_used.abs_diff(*revm_gas_used)
                                )?;
                            }
                            if let Some((levm_gas_refunded, revm_gas_refunded)) =
                                &execution_report.gas_refunded_mismatch
                            {
                                writeln!(
                                    f,
                                    "Gas refunded mismatch: LEVM: {levm_gas_refunded}, REVM: {revm_gas_refunded} (diff: {})",
                                    levm_gas_refunded.abs_diff(*revm_gas_refunded)
                                )?;
                            }
                            if let Some((levm_result, revm_error)) =
                                &execution_report.re_runner_error
                            {
                                writeln!(
                                    f,
                                    "Re-run error: LEVM: {levm_result:?}, REVM: {revm_error}",
                                )?;
                            }
                        }

                        if let Some(account_update) = re_run_report
                            .account_updates_report
                            .get(&(*failed_vector, *fork))
                        {
                            writeln!(f, "{}", &account_update.to_string())?;
                        } else {
                            writeln!(f, "No account updates report found. Account update reports are only generated for tests that failed at the post-state validation stage.")?;
                        }
                    } else {
                        writeln!(f, "No re-run report found. Re-run reports are only generated for tests that failed at the post-state validation stage.")?;
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
    ) {
        self.failed_vectors.insert(
            failed_vector,
            EFTestRunnerError::FailedToEnsurePostState(transaction_report, reason),
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
            writeln!(
                f,
                "================ WARNING: REVM *fails* this tests ==================="
            )?
        }
        if self.levm_post_state_root != self.revm_post_state_root {
            writeln!(
                f,
                "Post-state roots mismatch: LEVM: {levm_post_state_root:#x}, REVM: {revm_post_state_root:#x}",
                levm_post_state_root = self.levm_post_state_root,
                revm_post_state_root = self.revm_post_state_root
            )?;
        } else {
            writeln!(
                f,
                "Post-state roots match to: {levm_post_state_root:#x}",
                levm_post_state_root = self.levm_post_state_root
            )?;
        }
        writeln!(f, "Account Updates:")?;
        for levm_updated_account_only in self.levm_updated_accounts_only.iter() {
            writeln!(f, "  {levm_updated_account_only:#x}:")?;
            writeln!(f, "    Was updated in LEVM but not in REVM")?;
            let initial_account = self
                .initial_accounts
                .get(levm_updated_account_only)
                .cloned()
                .unwrap_or_default();
            let updated_account_update = self
                .levm_account_updates
                .iter()
                .find(|account_update| &account_update.address == levm_updated_account_only)
                .unwrap();
            let updated_account_storage: HashMap<H256, StorageSlot> = updated_account_update
                .added_storage
                .iter()
                .map(|(key, value)| {
                    let storage_slot = StorageSlot {
                        original_value: initial_account
                            .storage
                            .get(key)
                            .cloned()
                            .unwrap_or_default(),
                        current_value: *value,
                    };
                    (*key, storage_slot)
                })
                .collect();
            let updated_account_info = updated_account_update.info.clone().unwrap();
            let updated_account = Account::new(
                updated_account_info.balance,
                updated_account_update.code.clone().unwrap_or_default(),
                updated_account_info.nonce,
                HashMap::new(),
            );
            let mut updates = 0;
            if initial_account.info.balance != updated_account.info.balance {
                writeln!(
                    f,
                    "      Balance updated: {initial_balance} -> {updated_balance}",
                    initial_balance = initial_account.info.balance,
                    updated_balance = updated_account.info.balance
                )?;
                updates += 1;
            }
            if initial_account.info.nonce != updated_account.info.nonce {
                writeln!(
                    f,
                    "      Nonce updated: {initial_nonce} -> {updated_nonce}",
                    initial_nonce = initial_account.info.nonce,
                    updated_nonce = updated_account.info.nonce
                )?;
                updates += 1;
            }
            if initial_account.code != updated_account.code {
                writeln!(
                    f,
                    "      Code updated: {initial_code}, {updated_code}",
                    initial_code = if initial_account.code.is_empty() {
                        "was empty".to_string()
                    } else {
                        hex::encode(&initial_account.code)
                    },
                    updated_code = hex::encode(&updated_account.code)
                )?;
                updates += 1;
            }
            for (added_storage_address, added_storage_slot) in updated_account_storage.iter() {
                writeln!(
                    f,
                    "      Storage slot added: {added_storage_address}: {} -> {}",
                    added_storage_slot.original_value, added_storage_slot.current_value
                )?;
                updates += 1;
            }
            if updates == 0 {
                writeln!(f, "      No changes")?;
            }
        }
        for revm_updated_account_only in self.revm_updated_accounts_only.iter() {
            writeln!(f, "  {revm_updated_account_only:#x}:")?;
            writeln!(f, "    Was updated in REVM but not in LEVM")?;
            let initial_account = self
                .initial_accounts
                .get(revm_updated_account_only)
                .cloned()
                .unwrap_or_default();
            let updated_account_update = self
                .revm_account_updates
                .iter()
                .find(|account_update| &account_update.address == revm_updated_account_only)
                .unwrap();
            let updated_account_storage: HashMap<H256, StorageSlot> = updated_account_update
                .added_storage
                .iter()
                .map(|(key, value)| {
                    let storage_slot = StorageSlot {
                        original_value: initial_account
                            .storage
                            .get(key)
                            .cloned()
                            .unwrap_or_default(),
                        current_value: *value,
                    };
                    (*key, storage_slot)
                })
                .collect();
            let Some(updated_account_info) = updated_account_update.info.clone() else {
                continue;
            };
            let updated_account = Account::new(
                updated_account_info.balance,
                updated_account_update.code.clone().unwrap_or_default(),
                updated_account_info.nonce,
                HashMap::new(),
            );
            let mut updates = 0;
            if initial_account.info.balance != updated_account.info.balance {
                writeln!(
                    f,
                    "      Balance updated: {initial_balance} -> {updated_balance}",
                    initial_balance = initial_account.info.balance,
                    updated_balance = updated_account.info.balance
                )?;
                updates += 1;
            }
            if initial_account.info.nonce != updated_account.info.nonce {
                writeln!(
                    f,
                    "      Nonce updated: {initial_nonce} -> {updated_nonce}",
                    initial_nonce = initial_account.info.nonce,
                    updated_nonce = updated_account.info.nonce
                )?;
                updates += 1;
            }
            if initial_account.code != updated_account.code {
                writeln!(
                    f,
                    "      Code updated: {initial_code}, {updated_code}",
                    initial_code = if initial_account.code.is_empty() {
                        "was empty".to_string()
                    } else {
                        hex::encode(&initial_account.code)
                    },
                    updated_code = hex::encode(&updated_account.code)
                )?;
                updates += 1;
            }
            for (added_storage_address, added_storage_slot) in updated_account_storage.iter() {
                writeln!(
                    f,
                    "      Storage slot added: {added_storage_address}: {} -> {}",
                    added_storage_slot.original_value, added_storage_slot.current_value
                )?;
                updates += 1;
            }
            if updates == 0 {
                writeln!(f, "      No changes")?;
            }
        }
        for shared_updated_account in self.shared_updated_accounts.iter() {
            writeln!(f, "  {shared_updated_account:#x}:")?;

            writeln!(f, "    Was updated in both LEVM and REVM")?;

            let levm_updated_account = self
                .levm_account_updates
                .iter()
                .find(|account_update| &account_update.address == shared_updated_account)
                .unwrap();
            let revm_updated_account = self
                .revm_account_updates
                .iter()
                .find(|account_update| &account_update.address == shared_updated_account)
                .unwrap();

            let mut diffs = 0;
            match (levm_updated_account.removed, revm_updated_account.removed) {
                (true, false) => {
                    writeln!(f, "      Removed in LEVM but not in REVM")?;
                    diffs += 1;
                }
                (false, true) => {
                    writeln!(f, "      Removed in REVM but not in LEVM")?;
                    diffs += 1;
                }
                // Account was removed in both VMs.
                (false, false) | (true, true) => {}
            }

            match (&levm_updated_account.code, &revm_updated_account.code) {
                (None, Some(revm_account_code)) => {
                    if **revm_account_code != *b"" {
                        writeln!(f, "      Has code in REVM but not in LEVM")?;
                        writeln!(f, "      REVM code: {}", hex::encode(revm_account_code))?;
                        diffs += 1;
                    }
                }
                (Some(levm_account_code), None) => {
                    if **levm_account_code != *b"" {
                        writeln!(f, "      Has code in LEVM but not in REVM")?;
                        writeln!(f, "      LEVM code: {}", hex::encode(levm_account_code))?;
                        diffs += 1;
                    }
                }
                (Some(levm_account_code), Some(revm_account_code)) => {
                    if levm_account_code != revm_account_code {
                        writeln!(f,
                            "      Code mismatch: LEVM: {levm_account_code}, REVM: {revm_account_code}",
                            levm_account_code = hex::encode(levm_account_code),
                            revm_account_code = hex::encode(revm_account_code)
                        )?;
                    }
                }
                (None, None) => {}
            }

            match (&levm_updated_account.info, &revm_updated_account.info) {
                (None, Some(_)) => {
                    writeln!(
                        f,
                        "      Account {shared_updated_account:#x} has info in REVM but not in LEVM"
                    )?;
                    diffs += 1;
                }
                (Some(levm_account_info), Some(revm_account_info)) => {
                    if levm_account_info.balance != revm_account_info.balance {
                        writeln!(f,
                            "      Balance mismatch: LEVM: {levm_account_balance}, REVM: {revm_account_balance}",
                            levm_account_balance = levm_account_info.balance,
                            revm_account_balance = revm_account_info.balance
                        )?;
                        diffs += 1;
                    }
                    if levm_account_info.nonce != revm_account_info.nonce {
                        writeln!(f,
                                "      Nonce mismatch: LEVM: {levm_account_nonce}, REVM: {revm_account_nonce}",
                                levm_account_nonce = levm_account_info.nonce,
                                revm_account_nonce = revm_account_info.nonce
                        )?;
                        diffs += 1;
                    }
                }
                // We ignore the case (Some(_), None) because we always add the account info to the account update.
                (Some(_), None) | (None, None) => {}
            }

            for (levm_key, levm_value) in levm_updated_account.added_storage.iter() {
                if let Some(revm_value) = revm_updated_account.added_storage.get(levm_key) {
                    if revm_value != levm_value {
                        writeln!(f, "      Storage slot added {levm_key} -> value mismatch REVM: {revm_value} LEVM: {levm_value}")?;
                        diffs += 1;
                    }
                } else {
                    writeln!(f, "      Storage slot added key is in LEVM but not in REVM {levm_key} -> {levm_value}")?;
                    diffs += 1;
                }
            }
            for (revm_key, revm_value) in revm_updated_account.added_storage.iter() {
                if !levm_updated_account.added_storage.contains_key(revm_key) {
                    writeln!(
                        f,
                        "      Storage slot added key is in REVM but not in LEVM: {revm_key} -> {revm_value}"
                    )?;
                    diffs += 1;
                }
            }

            if diffs == 0 {
                writeln!(f, "      Same changes")?;
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
        revm_error: EVMError<StoreError>,
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
