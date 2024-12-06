use crate::{
    report::{self, format_duration_as_mm_ss, EFTestReport, TestReRunReport},
    types::EFTest,
    utils::{spinner_success_or_print, spinner_update_text_or_print},
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
    #[arg(short, long, value_name = "TESTS", use_value_delimiter = true)]
    pub tests: Vec<String>,
    #[arg(short, long, value_name = "SUMMARY", default_value = "false")]
    pub summary: bool,
    #[arg(long, value_name = "SKIP", use_value_delimiter = true)]
    pub skip: Vec<String>,
    #[arg(long, value_name = "SPINNER", default_value = "false")]
    pub spinner: bool, // Replaces prints for spinner, but execution is slower.
    #[arg(long, value_name = "VERBOSE", default_value = "false")]
    pub verbose: bool,
}

pub fn run_ef_tests(
    ef_tests: Vec<EFTest>,
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let mut reports = report::load()?;
    if reports.is_empty() {
        run_with_levm(&mut reports, &ef_tests, opts)?;
    }
    if opts.summary {
        return Ok(());
    }
    re_run_with_revm(&mut reports, &ef_tests, opts)?;
    write_report(&reports)
}

fn run_with_levm(
    reports: &mut Vec<EFTestReport>,
    ef_tests: &[EFTest],
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let levm_run_time = std::time::Instant::now();
    let mut levm_run_spinner = Spinner::new(
        Dots,
        report::progress(reports, levm_run_time.elapsed()),
        Color::Cyan,
    );
    if !opts.spinner {
        levm_run_spinner.stop();
    }
    for test in ef_tests.iter() {
        if !opts.spinner && opts.verbose {
            println!("Running test: {:?}", test.name);
        }
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
        spinner_update_text_or_print(
            &mut levm_run_spinner,
            report::progress(reports, levm_run_time.elapsed()),
            opts.spinner,
        );
    }
    spinner_success_or_print(
        &mut levm_run_spinner,
        report::progress(reports, levm_run_time.elapsed()),
        opts.spinner,
    );

    if opts.summary {
        report::write_summary_for_slack(reports)?;
        report::write_summary_for_github(reports)?;
    }

    let mut summary_spinner = Spinner::new(Dots, "Loading summary...".to_owned(), Color::Cyan);
    if !opts.spinner {
        summary_spinner.stop();
    }
    spinner_success_or_print(
        &mut summary_spinner,
        report::summary_for_shell(reports),
        opts.spinner,
    );

    Ok(())
}

/// ### Runs all tests with REVM
/// **Note:** This is not used in the current implementation because we only run with REVM the tests that failed with LEVM so that execution time is minimized.
fn _run_with_revm(
    reports: &mut Vec<EFTestReport>,
    ef_tests: &[EFTest],
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let revm_run_time = std::time::Instant::now();
    let mut revm_run_spinner = Spinner::new(
        Dots,
        "Running all tests with REVM...".to_owned(),
        Color::Cyan,
    );
    if !opts.spinner {
        revm_run_spinner.stop();
    }
    for (idx, test) in ef_tests.iter().enumerate() {
        if !opts.spinner && opts.verbose {
            println!("Running test: {:?}", test.name);
        }
        let total_tests = ef_tests.len();
        spinner_update_text_or_print(
            &mut revm_run_spinner,
            format!(
                "{} {}/{total_tests} - {}",
                "Running all tests with REVM".bold(),
                idx + 1,
                format_duration_as_mm_ss(revm_run_time.elapsed())
            ),
            opts.spinner,
        );
        let ef_test_report = match revm_runner::_run_ef_test_revm(test) {
            Ok(ef_test_report) => ef_test_report,
            Err(EFTestRunnerError::Internal(err)) => return Err(EFTestRunnerError::Internal(err)),
            non_internal_errors => {
                return Err(EFTestRunnerError::Internal(InternalError::FirstRunInternal(format!(
                    "Non-internal error raised when executing revm. This should not happen: {non_internal_errors:?}",
                ))))
            }
        };
        reports.push(ef_test_report);
        spinner_update_text_or_print(
            &mut revm_run_spinner,
            report::progress(reports, revm_run_time.elapsed()),
            opts.spinner,
        );
    }
    spinner_success_or_print(
        &mut revm_run_spinner,
        format!(
            "Ran all tests with REVM in {}",
            format_duration_as_mm_ss(revm_run_time.elapsed())
        ),
        opts.spinner,
    );
    Ok(())
}

fn re_run_with_revm(
    reports: &mut [EFTestReport],
    ef_tests: &[EFTest],
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let revm_run_time = std::time::Instant::now();
    let mut revm_run_spinner = Spinner::new(
        Dots,
        "Running failed tests with REVM...".to_owned(),
        Color::Cyan,
    );
    if !opts.spinner {
        revm_run_spinner.stop();
    }
    let failed_tests = reports.iter().filter(|report| !report.passed()).count();

    // Iterate only over failed tests
    for (idx, failed_test_report) in reports
        .iter_mut()
        .filter(|report| !report.passed())
        .enumerate()
    {
        if !opts.spinner && opts.verbose {
            println!("Running test: {:?}", failed_test_report.name);
        }
        spinner_update_text_or_print(
            &mut revm_run_spinner,
            format!(
                "{} {}/{failed_tests} - {}",
                "Re-running failed tests with REVM".bold(),
                idx + 1,
                format_duration_as_mm_ss(revm_run_time.elapsed())
            ),
            opts.spinner,
        );

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
    spinner_success_or_print(
        &mut revm_run_spinner,
        format!(
            "Re-ran failed tests with REVM in {}",
            format_duration_as_mm_ss(revm_run_time.elapsed())
        ),
        opts.spinner,
    );
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
