use crate::{
    report::{self, format_duration_as_mm_ss, EFTestReport, TestReRunReport},
    types::EFTest,
};
use clap::Parser;
use colored::Colorize;
use ethrex_levm::errors::{TransactionReport, VMError};
use ethrex_vm::SpecId;
use serde::{Deserialize, Serialize};
use spinoff::{spinners::Dots, Color, Spinner};

pub mod levm_runner;
pub mod revm_runner;

#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum EFTestRunnerError {
    #[error("VM initialization failed: {0}")]
    VMInitializationFailed(String),
    #[error("Transaction execution failed when it was not expected to fail: {0}")]
    ExecutionFailedUnexpectedly(VMError),
    #[error("Failed to ensure post-state: {0}")]
    FailedToEnsurePreState(String),
    #[error("Failed to ensure post-state: {1}")]
    FailedToEnsurePostState(TransactionReport, String),
    #[error("VM run mismatch: {0}")]
    VMExecutionMismatch(String),
    #[error("This is a bug: {0}")]
    Internal(#[from] InternalError),
}

#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum InternalError {
    #[error("First run failed unexpectedly: {0}")]
    FirstRunInternal(String),
    #[error("Re-runner failed unexpectedly: {0}")]
    ReRunInternal(String, TestReRunReport),
    #[error("Main runner failed unexpectedly: {0}")]
    MainRunnerInternal(String),
}

#[derive(Parser)]
pub struct EFTestRunnerOptions {
    #[arg(short, long, value_name = "FORK", default_value = "Cancun")]
    pub fork: Vec<SpecId>,
    #[arg(short, long, value_name = "TESTS")]
    pub tests: Vec<String>,
}

pub fn run_ef_tests(
    ef_tests: Vec<EFTest>,
    _opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let mut reports = report::load()?;
    if reports.is_empty() {
        run_with_levm(&mut reports, &ef_tests)?;
    }
    re_run_with_revm(&mut reports, &ef_tests)?;
    write_report(&reports)
}

fn run_with_levm(
    reports: &mut Vec<EFTestReport>,
    ef_tests: &[EFTest],
) -> Result<(), EFTestRunnerError> {
    let levm_run_time = std::time::Instant::now();
    let mut levm_run_spinner = Spinner::new(
        Dots,
        report::progress(reports, levm_run_time.elapsed()),
        Color::Cyan,
    );
    for test in ef_tests.iter() {
        let ef_test_report = match levm_runner::run_ef_test(test) {
            Ok(ef_test_report) => ef_test_report,
            Err(EFTestRunnerError::Internal(err)) => return Err(EFTestRunnerError::Internal(err)),
            non_internal_errors => {
                return Err(EFTestRunnerError::Internal(InternalError::FirstRunInternal(format!(
                    "Non-internal error raised when executing levm. This should not happen: {non_internal_errors:?}",
                ))))
            }
        };
        reports.push(ef_test_report);
        levm_run_spinner.update_text(report::progress(reports, levm_run_time.elapsed()));
    }
    levm_run_spinner.success(&report::progress(reports, levm_run_time.elapsed()));

    let mut summary_spinner = Spinner::new(Dots, "Loading summary...".to_owned(), Color::Cyan);
    summary_spinner.success(&report::summary(reports));
    Ok(())
}

fn re_run_with_revm(
    reports: &mut [EFTestReport],
    ef_tests: &[EFTest],
) -> Result<(), EFTestRunnerError> {
    let revm_run_time = std::time::Instant::now();
    let mut revm_run_spinner = Spinner::new(
        Dots,
        "Running failed tests with REVM...".to_owned(),
        Color::Cyan,
    );
    let failed_tests = reports.iter().filter(|report| !report.passed()).count();
    for (idx, failed_test_report) in reports.iter_mut().enumerate() {
        if failed_test_report.passed() {
            continue;
        }
        revm_run_spinner.update_text(format!(
            "{} {}/{failed_tests} - {}",
            "Re-running failed tests with REVM".bold(),
            idx + 1,
            format_duration_as_mm_ss(revm_run_time.elapsed())
        ));
        match revm_runner::re_run_failed_ef_test(
            ef_tests
                .iter()
                .find(|test| test._info.generated_test_hash == failed_test_report.test_hash)
                .unwrap(),
            failed_test_report,
        ) {
            Ok(re_run_report) => {
                failed_test_report.register_re_run_report(re_run_report.clone());
            }
            Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(reason, re_run_report))) => {
                write_report(reports)?;
                cache_re_run(reports)?;
                return Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(
                    reason,
                    re_run_report,
                )))
            },
            non_re_run_internal_errors => {
                return Err(EFTestRunnerError::Internal(InternalError::MainRunnerInternal(format!(
                    "Non-internal error raised when executing revm. This should not happen: {non_re_run_internal_errors:?}"
                ))))
            }
        }
    }
    revm_run_spinner.success(&format!(
        "Re-ran failed tests with REVM in {}",
        format_duration_as_mm_ss(revm_run_time.elapsed())
    ));
    Ok(())
}

fn write_report(reports: &[EFTestReport]) -> Result<(), EFTestRunnerError> {
    let mut report_spinner = Spinner::new(Dots, "Loading report...".to_owned(), Color::Cyan);
    let report_file_path = report::write(reports)?;
    report_spinner.success(&format!("Report written to file {report_file_path:?}").bold());
    Ok(())
}

fn cache_re_run(reports: &[EFTestReport]) -> Result<(), EFTestRunnerError> {
    let mut cache_spinner = Spinner::new(Dots, "Caching re-run...".to_owned(), Color::Cyan);
    let cache_file_path = report::cache(reports)?;
    cache_spinner.success(&format!("Re-run cached to file {cache_file_path:?}").bold());
    Ok(())
}
