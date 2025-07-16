use ef_tests_state::runner_v2::{error::RunnerError, parser::parse_dir, runner::run_tests};

#[tokio::main]
pub async fn main() -> Result<(), RunnerError> {
    let test_path = "./runner_v2/failing_tests/";
    println!("Parsing test files...");
    let tests = parse_dir(test_path.into())?;
    println!("Finalized parsing. Executing tests...");
    run_tests(tests).await?;
    println!("Tests finalized running.
    Find successful tests (if any) report at: './runner_v2/success_report.txt'.
    Find failing    tests (if any) report at: './runner_v2/failure_report.txt'.
    ");
    Ok(())
}
