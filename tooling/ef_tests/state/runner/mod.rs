use std::collections::BTreeMap;

use crate::report::EFTestsReport;
use crate::{
    parser::SPECIFIC_IGNORED_TESTS,
    report::{self, EFTestReport},
    types::EFTest,
};
use clap::Parser;
use colored::Colorize;
use ethrex_common::Address;
use ethrex_common::types::Fork;
use ethrex_levm::account::LevmAccount;
use ethrex_levm::errors::{ExecutionReport, VMError};
use serde::{Deserialize, Serialize};
use spinoff::{Color, Spinner, spinners::Dots};

pub mod levm_runner;

#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum EFTestRunnerError {
    #[error("VM initialization failed: {0}")]
    VMInitializationFailed(String),
    #[error("Transaction execution failed when it was not expected to fail: {0}")]
    ExecutionFailedUnexpectedly(VMError),
    #[error("Failed to ensure pre-state: {0}")]
    FailedToEnsurePreState(String),
    #[error("Failed to ensure post-state: {1}")]
    FailedToEnsurePostState(Box<ExecutionReport>, String, BTreeMap<Address, LevmAccount>),
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
        value_parser=parse_fork,
        default_value = "Paris,Shanghai,Cancun,Prague,Osaka,Amsterdam"
    )]
    pub forks: Option<Vec<Fork>>,
    /// For running specific .json files
    #[arg(short, long, value_name = "TESTS", value_delimiter = ',')]
    pub tests: Vec<String>,
    /// For running tests with a specific name
    #[arg(long, value_name = "SPECIFIC_TESTS", value_delimiter = ',')]
    pub specific_tests: Vec<String>,
    /// For printing only the failure summary instead of writing a report file.
    #[arg(short, long, value_name = "SUMMARY", default_value = "false")]
    pub summary: bool,
    #[arg(long, value_name = "SKIP", value_delimiter = ',')]
    pub skip: Vec<String>,
    /// For providing more detailed information
    #[arg(long, value_name = "VERBOSE", default_value = "false")]
    pub verbose: bool,
    /// For running particular tests that have their specified paths listed with the tests flag.
    #[arg(long, value_name = "PATHS", default_value = "false")]
    pub paths: bool,
}

fn parse_fork(value: &str) -> Result<Fork, String> {
    match value {
        "Frontier" => Ok(Fork::Frontier),
        "FrontierThawing" => Ok(Fork::FrontierThawing),
        "Homestead" => Ok(Fork::Homestead),
        "DaoFork" => Ok(Fork::DaoFork),
        "Tangerine" => Ok(Fork::Tangerine),
        "SpuriousDragon" => Ok(Fork::SpuriousDragon),
        "Byzantium" => Ok(Fork::Byzantium),
        "Constantinople" => Ok(Fork::Constantinople),
        "Petersburg" => Ok(Fork::Petersburg),
        "Istanbul" => Ok(Fork::Istanbul),
        "MuirGlacier" => Ok(Fork::MuirGlacier),
        "Berlin" => Ok(Fork::Berlin),
        "London" => Ok(Fork::London),
        "ArrowGlacier" => Ok(Fork::ArrowGlacier),
        "GrayGlacier" => Ok(Fork::GrayGlacier),
        "Paris" | "Merge" => Ok(Fork::Paris),
        "Shanghai" => Ok(Fork::Shanghai),
        "Cancun" => Ok(Fork::Cancun),
        "Prague" => Ok(Fork::Prague),
        "Osaka" => Ok(Fork::Osaka),
        "Amsterdam" => Ok(Fork::Amsterdam),
        other => Err(format!("Unknown fork: {other}")),
    }
}

pub async fn run_ef_tests(
    ef_tests: Vec<EFTest>,
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let mut reports = Vec::new();
    run_with_levm(&mut reports, &ef_tests, opts).await?;
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
    if reports.iter().any(|r| !r.passed()) {
        return write_report(&reports);
    }
    let mut report_spinner = Spinner::new(Dots, "Loading report...".to_owned(), Color::Cyan);
    report_spinner.success(&format!("{}", "All tests passed!".bold()));
    Ok(())
}

async fn run_with_levm(
    reports: &mut Vec<EFTestReport>,
    ef_tests: &[EFTest],
    opts: &EFTestRunnerOptions,
) -> Result<(), EFTestRunnerError> {
    let levm_run_time = std::time::Instant::now();

    print!("{}", report::progress(reports, levm_run_time.elapsed()));

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
        print!("{}", report::progress(reports, levm_run_time.elapsed()));
    }
    print!("{}", report::progress(reports, levm_run_time.elapsed()));

    if opts.summary {
        report::write_summary_for_slack(reports)?;
        report::write_summary_for_github(reports)?;
    }

    println!("{}", "\nLoading summary...".to_owned());
    println!("{}", report::summary_for_shell(reports));

    Ok(())
}

fn write_report(reports: &[EFTestReport]) -> Result<(), EFTestRunnerError> {
    let mut report_spinner = Spinner::new(Dots, "Loading report...".to_owned(), Color::Cyan);
    let report_file_path = report::write(reports)?;
    report_spinner.success(&format!("Report written to file {report_file_path:?}").bold());
    Ok(())
}
