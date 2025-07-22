use std::env;

use ef_tests_state::runner_v2::{error::RunnerError, parser::parse_dir, runner::run_tests};

#[tokio::main]
pub async fn main() -> Result<(), RunnerError> {
    let args: Vec<String> = env::args().collect();
    let path = &args[1];
    println!("Parsing test files...");
    let tests = parse_dir(path.into())?;
    println!("Finished parsing. Executing tests...");
    run_tests(tests).await?;
    println!("Tests finished running. Find the report at: './runner_v2/runner_report.txt'");
    Ok(())
}
