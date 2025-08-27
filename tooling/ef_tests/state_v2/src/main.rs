use clap::Parser;
use ef_tests_statev2::modules::{
    error::RunnerError,
    parser::{RunnerOptions, parse_tests},
};

#[tokio::main]
pub async fn main() -> Result<(), RunnerError> {
    let mut runner_options = RunnerOptions::parse();
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
    Find successful tests (if any) report at: './success_report.txt'.
    Find failing    tests (if any) report at: './failure_report.txt'.
    "
    );
    Ok(())
}
