use std::collections::HashMap;

use crate::report::EFTestsReport;
use crate::{
    parser::SPECIFIC_IGNORED_TESTS,
    report::{self, EFTestReport, TestReRunReport, format_duration_as_mm_ss},
    types::EFTest,
};
use clap::Parser;
use colored::Colorize;
use ethrex_common::{Address, types::Account};
use ethrex_levm::errors::{ExecutionReport, VMError};
use ethrex_vm::SpecId;
use serde::{Deserialize, Serialize};
use spinoff::{Color, Spinner, spinners::Dots};

pub mod levm_runner;
pub mod revm_runner;

#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum EFTestRunnerError {
    #[error("VM initialization failed: {0}")]
    VMInitializationFailed(String),
    #[error("Transaction execution failed when it was not expected to fail: {0}")]
    ExecutionFailedUnexpectedly(VMError),
    #[error("Failed to ensure pre-state: {0}")]
    FailedToEnsurePreState(String),
    #[error("Failed to ensure post-state: {1}")]
    FailedToEnsurePostState(Box<ExecutionReport>, String, HashMap<Address, Account>),
    #[error("VM run mismatch: {0}")]
    VMExecutionMismatch(String),
    #[error("Exception does not match the expected: {0}")]
    ExpectedExceptionDoesNotMatchReceived(String),
    #[error("EIP-7702 should not be a create type")]
    EIP7702ShouldNotBeCreateType,
    #[error("This is a bug: {0}")]
    Internal(#[from] InternalError),
    #[error("Failed to revert levm state after transaction error: {0}")]
    FailedToRevertLEVMState(String),
    #[error("Tests failed")]
    TestsFailed,
}

#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum InternalError {
    #[error("First run failed unexpectedly: {0}")]
    FirstRunInternal(String),
    #[error("Re-runner failed unexpectedly: {0}")]
    ReRunInternal(String, TestReRunReport),
    #[error("Main runner failed unexpectedly: {0}")]
    MainRunnerInternal(String),
    #[error("{0}")]
    Custom(String),
}

#[derive(Parser, Debug, Default)]
pub struct EFTestRunnerOptions {
    /// For running tests of specific forks.
    #[arg(
        long,
        value_name = "FORK",
        value_delimiter = ',',
        default_value = "Merge,Shanghai,Cancun,Prague"
    )]
    pub forks: Option<Vec<SpecId>>,
    /// For running specific .json files
    #[arg(short, long, value_name = "TESTS", value_delimiter = ',')]
    pub tests: Vec<String>,
    /// For running tests with a specific name
    #[arg(long, value_name = "SPECIFIC_TESTS", value_delimiter = ',')]
    pub specific_tests: Vec<String>,
    /// For running tests only with LEVM without the REVM re-run.
    #[arg(short, long, value_name = "SUMMARY", default_value = "false")]
    pub summary: bool,
    #[arg(long, value_name = "SKIP", value_delimiter = ',')]
    pub skip: Vec<String>,
    /// For providing more detailed information
    #[arg(long, value_name = "VERBOSE", default_value = "false")]
    pub verbose: bool,
    /// For running tests ONLY with revm
    #[arg(long, value_name = "REVM", default_value = "false")]
    pub revm: bool,
}

pub async fn run_ef_tests(
    ef_tests: Vec<EFTest>,
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let mut reports = report::load()?;
    if reports.is_empty() {
        if opts.revm {
            run_with_revm(&mut reports, &ef_tests, opts).await?;
            return Ok(());
        } else {
            run_with_levm(&mut reports, &ef_tests, opts).await?;
        }
    }
    if opts.summary {
        if reports.iter().any(|r| !r.passed()) {
            println!(
                "{}",
                EFTestsReport(reports.iter().filter(|&r| !r.passed()).cloned().collect(),)
            );
            return Err(EFTestRunnerError::TestsFailed);
        }
        return Ok(());
    }
    re_run_with_revm(&mut reports, &ef_tests, opts).await?;
    write_report(&reports)
}

async fn run_with_levm(
    reports: &mut Vec<EFTestReport>,
    ef_tests: &[EFTest],
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let levm_run_time = std::time::Instant::now();

    println!("{}", report::progress(reports, levm_run_time.elapsed()));

    for test in ef_tests.iter() {
        let is_not_specific = !opts.specific_tests.is_empty()
            && !opts
                .specific_tests
                .iter()
                .any(|name| test.name.contains(name));
        let is_ignored = SPECIFIC_IGNORED_TESTS
            .iter()
            .any(|skip| test.name.contains(skip));

        // Skip tests that are not specific (if specific tests were indicated) and ignored ones.
        if is_not_specific || is_ignored {
            continue;
        }
        if opts.verbose {
            println!("Running test: {:?}", test.name);
        }
        let ef_test_report = match levm_runner::run_ef_test(test).await {
            Ok(ef_test_report) => ef_test_report,
            Err(EFTestRunnerError::Internal(err)) => return Err(EFTestRunnerError::Internal(err)),
            non_internal_errors => {
                return Err(EFTestRunnerError::Internal(
                    InternalError::FirstRunInternal(format!(
                        "Non-internal error raised when executing levm. This should not happen: {non_internal_errors:?}",
                    )),
                ));
            }
        };
        reports.push(ef_test_report);
        println!("{}", report::progress(reports, levm_run_time.elapsed()));
    }
    println!("{}", report::progress(reports, levm_run_time.elapsed()));

    if opts.summary {
        report::write_summary_for_slack(reports)?;
        report::write_summary_for_github(reports)?;
    }

    println!("{}", "Loading summary...".to_owned());
    println!("{}", report::summary_for_shell(reports));

    Ok(())
}

/// ### Runs all tests with REVM
async fn run_with_revm(
    reports: &mut Vec<EFTestReport>,
    ef_tests: &[EFTest],
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let revm_run_time = std::time::Instant::now();
    println!("{}", "Running all tests with REVM...".to_owned());

    for (idx, test) in ef_tests.iter().enumerate() {
        if opts.verbose {
            println!("Running test: {:?}", test.name);
        }
        let total_tests = ef_tests.len();
        println!(
            "{} {}/{total_tests} - {}",
            "Running all tests with REVM".bold(),
            idx + 1,
            format_duration_as_mm_ss(revm_run_time.elapsed())
        );
        let ef_test_report = match revm_runner::_run_ef_test_revm(test).await {
            Ok(ef_test_report) => ef_test_report,
            Err(EFTestRunnerError::Internal(err)) => return Err(EFTestRunnerError::Internal(err)),
            non_internal_errors => {
                return Err(EFTestRunnerError::Internal(
                    InternalError::FirstRunInternal(format!(
                        "Non-internal error raised when executing revm. This should not happen: {non_internal_errors:?}",
                    )),
                ));
            }
        };
        reports.push(ef_test_report);
        println!("{}", report::progress(reports, revm_run_time.elapsed()));
    }
    println!(
        "Ran all tests with REVM in {}",
        format_duration_as_mm_ss(revm_run_time.elapsed())
    );
    Ok(())
}

async fn re_run_with_revm(
    reports: &mut [EFTestReport],
    ef_tests: &[EFTest],
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let revm_run_time = std::time::Instant::now();
    println!("{}", "Running failed tests with REVM...".to_owned());
    let failed_tests = reports.iter().filter(|report| !report.passed()).count();

    // Iterate only over failed tests
    for (idx, failed_test_report) in reports
        .iter_mut()
        .filter(|report| !report.passed())
        .enumerate()
    {
        if opts.verbose {
            println!("Running test: {:?}", failed_test_report.name);
        }
        println!(
            "{} {}/{failed_tests} - {}",
            "Re-running failed tests with REVM".bold(),
            idx + 1,
            format_duration_as_mm_ss(revm_run_time.elapsed())
        );

        match revm_runner::re_run_failed_ef_test(
            ef_tests
                .iter()
                .find(|test| {
                    let hash = test
                        ._info
                        .generated_test_hash
                        .or(test._info.hash)
                        .unwrap_or_default();

                    let failed_hash = failed_test_report.test_hash;

                    hash == failed_hash && test.name == failed_test_report.name
                })
                .unwrap(),
            failed_test_report,
        )
        .await
        {
            Ok(re_run_report) => {
                failed_test_report.register_re_run_report(re_run_report.clone());
            }
            Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(
                reason,
                re_run_report,
            ))) => {
                write_report(reports)?;
                cache_re_run(reports)?;
                return Err(EFTestRunnerError::Internal(InternalError::ReRunInternal(
                    reason,
                    re_run_report,
                )));
            }
            non_re_run_internal_errors => {
                return Err(EFTestRunnerError::Internal(
                    InternalError::MainRunnerInternal(format!(
                        "Non-internal error raised when executing revm. This should not happen: {non_re_run_internal_errors:?}"
                    )),
                ));
            }
        }
    }
    println!(
        "Re-ran failed tests with REVM in {}",
        format_duration_as_mm_ss(revm_run_time.elapsed())
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
