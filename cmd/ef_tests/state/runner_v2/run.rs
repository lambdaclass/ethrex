use ef_tests_state::runner_v2::{error::RunnerError, parser::parse_dir, runner::run_tests};

#[tokio::main]
pub async fn main() -> Result<(), RunnerError> {
    let test_path = "./runner_v2/test_files";
    println!("Parsing test files...");
    let tests = parse_dir(test_path.into())?;
    println!("Finalized parsing. Executing tests...");
    run_tests(tests).await?;
    println!("Tests finalized running. Find the report at: './runner_v2/runner_report.txt'");
    Ok(())
}
