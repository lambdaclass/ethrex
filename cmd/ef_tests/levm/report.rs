use crate::runner::{EFTestRunnerError, InternalError};
use colored::Colorize;
use ethrex_core::{Address, H256};
use ethrex_levm::{
    errors::{TransactionReport, TxResult, VMError},
    Account, StorageSlot,
};
use ethrex_storage::{error::StoreError, AccountUpdate};
use ethrex_vm::SpecId;
use revm::primitives::{EVMError, ExecutionResult as RevmExecutionResult};
use serde::{Deserialize, Serialize};
use spinoff::{spinners::Dots, Color, Spinner};
use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Display},
    path::PathBuf,
    time::Duration,
};

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

pub fn summary(reports: &[EFTestReport]) -> String {
    let total_passed = reports.iter().filter(|report| report.passed()).count();
    let total_run = reports.len();
    let success_percentage = (total_passed as f64 / total_run as f64) * 100.0;
    format!(
        "Summary: {total_passed}/{total_run} ({success_percentage:.2}%)\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        fork_summary(reports, SpecId::CANCUN),
        fork_summary(reports, SpecId::SHANGHAI),
        fork_summary(reports, SpecId::HOMESTEAD),
        fork_summary(reports, SpecId::ISTANBUL),
        fork_summary(reports, SpecId::LONDON),
        fork_summary(reports, SpecId::BYZANTIUM),
        fork_summary(reports, SpecId::BERLIN),
        fork_summary(reports, SpecId::CONSTANTINOPLE),
        fork_summary(reports, SpecId::MERGE),
        fork_summary(reports, SpecId::FRONTIER),
    )
}

pub fn summary_pretty(reports: &[EFTestReport]) -> String {
    let total_passed = reports.iter().filter(|report| report.passed()).count();
    let total_run = reports.len();
    let success_percentage = (total_passed as f64 / total_run as f64) * 100.0;
    format!(
        "{} {}/{total_run} ({success_percentage:.2})\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "Summary:".bold(),
        if total_passed == total_run {
            format!("{}", total_passed).green()
        } else if total_passed > 0 {
            format!("{}", total_passed).yellow()
        } else {
            format!("{}", total_passed).red()
        },
        fork_summary_pretty(reports, SpecId::CANCUN),
        fork_summary_pretty(reports, SpecId::SHANGHAI),
        fork_summary_pretty(reports, SpecId::HOMESTEAD),
        fork_summary_pretty(reports, SpecId::ISTANBUL),
        fork_summary_pretty(reports, SpecId::LONDON),
        fork_summary_pretty(reports, SpecId::BYZANTIUM),
        fork_summary_pretty(reports, SpecId::BERLIN),
        fork_summary_pretty(reports, SpecId::CONSTANTINOPLE),
        fork_summary_pretty(reports, SpecId::MERGE),
        fork_summary_pretty(reports, SpecId::FRONTIER),
    )
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

pub const LEVM_EF_TESTS_SUMMARY_FILE_PATH: &str = "./levm_ef_tests_summary.txt";

pub fn write_summary(reports: &[EFTestReport]) -> Result<PathBuf, EFTestRunnerError> {
    let summary_file_path = PathBuf::from(LEVM_EF_TESTS_SUMMARY_FILE_PATH);
    std::fs::write(LEVM_EF_TESTS_SUMMARY_FILE_PATH, summary(reports)).map_err(|err| {
        EFTestRunnerError::Internal(InternalError::MainRunnerInternal(format!(
            "Failed to write summary to file: {err}"
        )))
    })?;
    Ok(summary_file_path)
}

pub const EF_TESTS_CACHE_FILE_PATH: &str = "./levm_ef_tests_cache.json";

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

pub fn format_duration_as_mm_ss(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

#[derive(Debug, Default, Clone)]
pub struct EFTestsReport(pub Vec<EFTestReport>);

impl Display for EFTestsReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total_passed = self.0.iter().filter(|report| report.passed()).count();
        let total_run = self.0.len();
        writeln!(
            f,
            "{} {}/{total_run}",
            "Summary:".bold(),
            if total_passed == total_run {
                format!("{}", total_passed).green()
            } else if total_passed > 0 {
                format!("{}", total_passed).yellow()
            } else {
                format!("{}", total_passed).red()
            },
        )?;
        writeln!(f)?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::CANCUN))?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::SHANGHAI))?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::HOMESTEAD))?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::ISTANBUL))?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::LONDON))?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::BYZANTIUM))?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::BERLIN))?;
        writeln!(
            f,
            "{}",
            fork_summary_pretty(&self.0, SpecId::CONSTANTINOPLE)
        )?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::MERGE))?;
        writeln!(f, "{}", fork_summary_pretty(&self.0, SpecId::FRONTIER))?;
        writeln!(f)?;
        writeln!(f, "{}", "Failed tests:".bold())?;
        writeln!(f)?;
        for report in self.0.iter() {
            if report.failed_vectors.is_empty() {
                continue;
            }
            writeln!(f, "{}", format!("Test: {}", report.name).bold())?;
            writeln!(f)?;
            for (failed_vector, error) in &report.failed_vectors {
                writeln!(
                    f,
                    "{} (data_index: {}, gas_limit_index: {}, value_index: {})",
                    "Vector:".bold(),
                    failed_vector.0,
                    failed_vector.1,
                    failed_vector.2
                )?;
                writeln!(f, "{} {}", "Error:".bold(), error.to_string().red())?;
                if let Some(re_run_report) = &report.re_run_report {
                    if let Some(execution_report) =
                        re_run_report.execution_report.get(failed_vector)
                    {
                        if let Some((levm_result, revm_result)) =
                            &execution_report.execution_result_mismatch
                        {
                            writeln!(
                                f,
                                "{}: LEVM: {levm_result:?}, REVM: {revm_result:?}",
                                "Execution result mismatch".bold()
                            )?;
                        }
                        if let Some((levm_gas_used, revm_gas_used)) =
                            &execution_report.gas_used_mismatch
                        {
                            writeln!(
                                f,
                                "{}: LEVM: {levm_gas_used}, REVM: {revm_gas_used} (diff: {})",
                                "Gas used mismatch".bold(),
                                levm_gas_used.abs_diff(*revm_gas_used)
                            )?;
                        }
                        if let Some((levm_gas_refunded, revm_gas_refunded)) =
                            &execution_report.gas_refunded_mismatch
                        {
                            writeln!(
                                f,
                                "{}: LEVM: {levm_gas_refunded}, REVM: {revm_gas_refunded} (diff: {})",
                                "Gas refunded mismatch".bold(), levm_gas_refunded.abs_diff(*revm_gas_refunded)
                            )?;
                        }
                        if let Some((levm_result, revm_error)) = &execution_report.re_runner_error {
                            writeln!(
                                f,
                                "{}: LEVM: {levm_result:?}, REVM: {revm_error}",
                                "Re-run error".bold()
                            )?;
                        }
                    }

                    if let Some(account_update) =
                        re_run_report.account_updates_report.get(failed_vector)
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
        Ok(())
    }
}

fn fork_summary(reports: &[EFTestReport], fork: SpecId) -> String {
    let fork_str: &str = fork.into();
    let fork_tests = reports.iter().filter(|report| report.fork == fork).count();
    let fork_passed_tests = reports
        .iter()
        .filter(|report| report.fork == fork && report.passed())
        .count();
    let fork_success_percentage = (fork_passed_tests as f64 / fork_tests as f64) * 100.0;
    format!("{fork_str}: {fork_passed_tests}/{fork_tests} ({fork_success_percentage:.2}%)")
}

fn fork_summary_pretty(reports: &[EFTestReport], fork: SpecId) -> String {
    let fork_str: &str = fork.into();
    let fork_tests = reports.iter().filter(|report| report.fork == fork).count();
    let fork_passed_tests = reports
        .iter()
        .filter(|report| report.fork == fork && report.passed())
        .count();
    let fork_success_percentage = (fork_passed_tests as f64 / fork_tests as f64) * 100.0;
    format!(
        "{}: {}/{fork_tests} ({fork_success_percentage}%)",
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EFTestReport {
    pub name: String,
    pub test_hash: H256,
    pub fork: SpecId,
    pub skipped: bool,
    pub failed_vectors: HashMap<TestVector, EFTestRunnerError>,
    pub re_run_report: Option<TestReRunReport>,
}

impl EFTestReport {
    pub fn new(name: String, test_hash: H256, fork: SpecId) -> Self {
        EFTestReport {
            name,
            test_hash,
            fork,
            ..Default::default()
        }
    }

    pub fn new_skipped() -> Self {
        EFTestReport {
            skipped: true,
            ..Default::default()
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
        transaction_report: TransactionReport,
        reason: String,
        failed_vector: TestVector,
    ) {
        self.failed_vectors.insert(
            failed_vector,
            EFTestRunnerError::FailedToEnsurePostState(transaction_report, reason),
        );
    }

    pub fn register_re_run_report(&mut self, re_run_report: TestReRunReport) {
        self.re_run_report = Some(re_run_report);
    }

    pub fn iter_failed(&self) -> impl Iterator<Item = (&TestVector, &EFTestRunnerError)> {
        self.failed_vectors.iter()
    }

    pub fn passed(&self) -> bool {
        self.failed_vectors.is_empty()
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AccountUpdatesReport {
    pub initial_accounts: HashMap<Address, Account>,
    pub levm_account_updates: Vec<AccountUpdate>,
    pub revm_account_updates: Vec<AccountUpdate>,
    pub levm_updated_accounts_only: HashSet<Address>,
    pub revm_updated_accounts_only: HashSet<Address>,
    pub shared_updated_accounts: HashSet<Address>,
}

impl fmt::Display for AccountUpdatesReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Account Updates:")?;
        for levm_updated_account_only in self.levm_updated_accounts_only.iter() {
            writeln!(f, "  {levm_updated_account_only:#x}:")?;
            writeln!(f, "{}", "    Was updated in LEVM but not in REVM".red())?;
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
            let updated_account_storage = updated_account_update
                .added_storage
                .iter()
                .map(|(key, value)| {
                    let storage_slot = StorageSlot {
                        original_value: *value,
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
                updated_account_storage,
            );
            let mut updates = 0;
            if initial_account.info.balance != updated_account.info.balance {
                writeln!(
                    f,
                    "{}",
                    format!(
                        "      Balance updated: {initial_balance} -> {updated_balance}",
                        initial_balance = initial_account.info.balance,
                        updated_balance = updated_account.info.balance
                    )
                    .red()
                )?;
                updates += 1;
            }
            if initial_account.info.nonce != updated_account.info.nonce {
                writeln!(
                    f,
                    "{}",
                    format!(
                        "      Nonce updated: {initial_nonce} -> {updated_nonce}",
                        initial_nonce = initial_account.info.nonce,
                        updated_nonce = updated_account.info.nonce
                    )
                    .red()
                )?;
                updates += 1;
            }
            if initial_account.info.bytecode != updated_account.info.bytecode {
                writeln!(
                    f,
                    "{}",
                    format!(
                        "      Code updated: {initial_code}, {updated_code}",
                        initial_code = hex::encode(&initial_account.info.bytecode),
                        updated_code = hex::encode(&updated_account.info.bytecode)
                    )
                    .red()
                )?;
                updates += 1;
            }
            for (added_storage_address, added_storage_slot) in updated_account.storage.iter() {
                writeln!(
                    f,
                    "{}",
                    format!(
                        "      Storage slot added: {added_storage_address}: {} -> {}",
                        added_storage_slot.original_value, added_storage_slot.current_value
                    )
                    .red()
                )?;
                updates += 1;
            }
            if updates == 0 {
                writeln!(f, "{}", "      No changes".green())?;
            }
        }
        for revm_updated_account_only in self.revm_updated_accounts_only.iter() {
            writeln!(f, "  {revm_updated_account_only:#x}:")?;
            writeln!(f, "{}", "    Was updated in REVM but not in LEVM".red())?;
        }
        for shared_updated_account in self.shared_updated_accounts.iter() {
            writeln!(f, "  {shared_updated_account:#x}:")?;

            writeln!(
                f,
                "{}",
                "    Was updated in both LEVM and REVM".to_string().green()
            )?;

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

            match (levm_updated_account.removed, revm_updated_account.removed) {
                (true, false) => {
                    writeln!(
                        f,
                        "{}",
                        "    Removed in LEVM but not in REVM".to_string().red()
                    )?;
                }
                (false, true) => {
                    writeln!(
                        f,
                        "{}",
                        "    Removed in REVM but not in LEVM".to_string().red()
                    )?;
                }
                // Account was removed in both VMs.
                (false, false) | (true, true) => {}
            }

            match (&levm_updated_account.code, &revm_updated_account.code) {
                (None, Some(_)) => {
                    writeln!(
                        f,
                        "{}",
                        "    Has code in REVM but not in LEVM".to_string().red()
                    )?;
                }
                (Some(_), None) => {
                    writeln!(
                        f,
                        "{}",
                        "    Has code in LEVM but not in REVM".to_string().red()
                    )?;
                }
                (Some(levm_account_code), Some(revm_account_code)) => {
                    if levm_account_code != revm_account_code {
                        writeln!(f,
                            "{}",
                            format!(
                                "    Code mismatch: LEVM: {levm_account_code}, REVM: {revm_account_code}",
                                levm_account_code = hex::encode(levm_account_code),
                                revm_account_code = hex::encode(revm_account_code)
                            )
                            .red()
                        )?;
                    }
                }
                (None, None) => {}
            }

            match (&levm_updated_account.info, &revm_updated_account.info) {
                (None, Some(_)) => {
                    writeln!(
                        f,
                        "{}",
                        format!("    Account {shared_updated_account:#x} has info in REVM but not in LEVM",)
                            .red()
                            .bold()
                    )?;
                }
                (Some(levm_account_info), Some(revm_account_info)) => {
                    if levm_account_info.balance != revm_account_info.balance {
                        writeln!(f,
                            "{}",
                            format!(
                                "    Balance mismatch: LEVM: {levm_account_balance}, REVM: {revm_account_balance}",
                                levm_account_balance = levm_account_info.balance,
                                revm_account_balance = revm_account_info.balance
                            )
                            .red()
                            .bold()
                        )?;
                    }
                    if levm_account_info.nonce != revm_account_info.nonce {
                        writeln!(f,
                            "{}",
                            format!(
                                "    Nonce mismatch: LEVM: {levm_account_nonce}, REVM: {revm_account_nonce}",
                                levm_account_nonce = levm_account_info.nonce,
                                revm_account_nonce = revm_account_info.nonce
                            )
                            .red()
                            .bold()
                        )?;
                    }
                }
                // We ignore the case (Some(_), None) because we always add the account info to the account update.
                (Some(_), None) | (None, None) => {}
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
    pub execution_report: HashMap<TestVector, TestReRunExecutionReport>,
    pub account_updates_report: HashMap<TestVector, AccountUpdatesReport>,
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
    ) {
        let value = Some((levm_result, revm_result));
        self.execution_report
            .entry(vector)
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
    ) {
        let value = Some((levm_gas_used, revm_gas_used));
        self.execution_report
            .entry(vector)
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
    ) {
        let value = Some((levm_gas_refunded, revm_gas_refunded));
        self.execution_report
            .entry(vector)
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
        report: AccountUpdatesReport,
    ) {
        self.account_updates_report.insert(vector, report);
    }

    pub fn register_re_run_failure(
        &mut self,
        vector: TestVector,
        levm_result: TxResult,
        revm_error: EVMError<StoreError>,
    ) {
        let value = Some((levm_result, revm_error.to_string()));
        self.execution_report
            .entry(vector)
            .and_modify(|report| {
                report.re_runner_error = value.clone();
            })
            .or_insert(TestReRunExecutionReport {
                re_runner_error: value,
                ..Default::default()
            });
    }
}
