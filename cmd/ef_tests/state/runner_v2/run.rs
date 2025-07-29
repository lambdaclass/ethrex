use clap::Parser;
use ef_tests_state::runner_v2::{
    error::RunnerError,
    parser::{RunnerOptions, parse_tests},
    runner::run_tests,
};

#[tokio::main]
pub async fn main() -> Result<(), RunnerError> {
    let mut runner_options = RunnerOptions::parse();
    println!("Runner options: {:#?}", runner_options);

    println!("\nParsing test files...");
    let tests = parse_tests(&mut runner_options)?;

    println!("\nFinished parsing. Executing tests...");
    run_tests(tests).await?;
    println!(
        "\nTests finished running.
    Find successful tests (if any) report at: './runner_v2/success_report.txt'.
    Find failing    tests (if any) report at: './runner_v2/failure_report.txt'.
    "
    );
    Ok(())
}
