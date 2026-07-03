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
pub async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Errors from a subcommand map to exit code 2 so that goevmlab can distinguish
    // a state-root mismatch (deliberate exit 1) from an actual internal failure.
    match cli.command {
        Some(Command::Statetest(opts)) => match statetest::run(opts).await {
            Ok(code) => code,
            Err(e) => {
                eprintln!("statetest error: {e:?}");
                ExitCode::from(2)
            }
        },
        None => match run_bulk(cli.runner).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e:?}");
                ExitCode::from(2)
            }
        },
    }
}

async fn run_bulk(mut runner_options: RunnerOptions) -> Result<(), RunnerError> {
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
    Ok(())
}
