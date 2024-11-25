use crate::runner::EFTestRunnerError;
use colored::Colorize;
use ethrex_core::Address;
use ethrex_levm::errors::{TransactionReport, TxResult, VMError};
use ethrex_storage::AccountUpdate;
use ethrex_vm::SpecId;
use revm::primitives::ExecutionResult as RevmExecutionResult;
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
    format!(
        "{} {}/{total_run}\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "Summary:".bold(),
        if total_passed == total_run {
            format!("{}", total_passed).green()
        } else if total_passed > 0 {
            format!("{}", total_passed).yellow()
        } else {
            format!("{}", total_passed).red()
        },
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

pub fn write(reports: Vec<EFTestReport>) -> Result<PathBuf, EFTestRunnerError> {
    let report_file_path = PathBuf::from("./levm_ef_tests_report.txt");
    let failed_test_reports = EFTestsReport(
        reports
            .into_iter()
            .filter(|report| !report.passed())
            .collect(),
    );
    std::fs::write(
        "./levm_ef_tests_report.txt",
        failed_test_reports.to_string(),
    )
    .map_err(|err| EFTestRunnerError::Internal(format!("Failed to write report to file: {err}")))?;
    Ok(report_file_path)
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
        writeln!(f, "{}", fork_summary(&self.0, SpecId::CANCUN))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::SHANGHAI))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::HOMESTEAD))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::ISTANBUL))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::LONDON))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::BYZANTIUM))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::BERLIN))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::CONSTANTINOPLE))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::MERGE))?;
        writeln!(f, "{}", fork_summary(&self.0, SpecId::FRONTIER))?;
        writeln!(f)?;
        writeln!(f, "{}", "Failed tests:".bold())?;
        writeln!(f)?;
        for report in self.0.iter() {
            if report.failed_vectors.is_empty() {
                continue;
            }
            writeln!(f, "{}", report.name.bold())?;
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
    format!(
        "{}: {}/{fork_tests}",
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

#[derive(Debug, Default, Clone)]
pub struct EFTestReport {
    pub name: String,
    pub fork: SpecId,
    pub skipped: bool,
    pub failed_vectors: HashMap<TestVector, EFTestRunnerError>,
    pub re_run_report: Option<TestReRunReport>,
}

impl EFTestReport {
    pub fn new(name: String, fork: SpecId) -> Self {
        EFTestReport {
            name,
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

#[derive(Debug, Default, Clone)]
pub struct AccountUpdatesReport {
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

#[derive(Debug, Default, Clone)]
pub struct TestReRunExecutionReport {
    pub execution_result_mismatch: Option<(TxResult, RevmExecutionResult)>,
    pub gas_used_mismatch: Option<(u64, u64)>,
    pub gas_refunded_mismatch: Option<(u64, u64)>,
}

#[derive(Debug, Default, Clone)]
pub struct TestReRunReport {
    pub execution_report: TestReRunExecutionReport,
    pub account_updates_report: HashMap<TestVector, AccountUpdatesReport>,
}

impl TestReRunReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_execution_result_mismatch(
        &mut self,
        levm_result: TxResult,
        revm_result: RevmExecutionResult,
    ) {
        self.execution_report.execution_result_mismatch = Some((levm_result, revm_result));
    }

    pub fn register_gas_used_mismatch(&mut self, levm_gas_used: u64, revm_gas_used: u64) {
        self.execution_report.gas_used_mismatch = Some((levm_gas_used, revm_gas_used));
    }

    pub fn register_gas_refunded_mismatch(
        &mut self,
        levm_gas_refunded: u64,
        revm_gas_refunded: u64,
    ) {
        self.execution_report.gas_refunded_mismatch = Some((levm_gas_refunded, revm_gas_refunded));
    }

    pub fn register_account_updates_report(
        &mut self,
        vector: TestVector,
        report: AccountUpdatesReport,
    ) {
        self.account_updates_report.insert(vector, report);
    }
}
