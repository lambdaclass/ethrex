#![allow(clippy::all)]

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ef_tests_statev2::modules::{
    error::RunnerError,
    parser::{RunnerOptions, parse_tests},
    statetest::{self, StatetestOptions},
};

#[derive(Parser, Debug)]
#[command(name = "ef-tests-state-v2")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Default (no subcommand): bulk-run the EF state-test suite.
    #[command(flatten)]
    runner: RunnerOptions,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a single EF state-test fixture and emit EIP-3155 trace + stateRoot to
    /// stderr. Designed for goevmlab differential fuzzing.
    Statetest(StatetestOptions),
}

#[tokio::main]
pub async fn main() -> Result<ExitCode, RunnerError> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Statetest(opts)) => statetest::run(opts).await,
        None => {
            let mut runner_options = cli.runner;
            println!("Runner options: {:#?}", runner_options);

            println!("\nParsing test files...");
            let tests = parse_tests(&mut runner_options)?;

            println!("\nFinished parsing. Executing tests...");

            if cfg!(feature = "block") {
                ef_tests_statev2::modules::block_runner::run_tests(tests.clone()).await?;
            } else {
                ef_tests_statev2::modules::runner::run_tests(tests).await?;
            }
            println!(
                "\nTests finished running.
    Find reports in the './reports' directory.
    "
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}
