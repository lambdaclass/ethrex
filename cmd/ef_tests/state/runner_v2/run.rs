use std::env;

use ef_tests_state::runner_v2::{error::RunnerError, parser::parse_dir, runner::run_tests};

#[tokio::main]
pub async fn main() -> Result<(), RunnerError> {
    let args: Vec<String> = env::args().collect();
    let path = &args[1];
    println!("\nParsing test files...");
    let tests = parse_dir(path.into())?;
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
