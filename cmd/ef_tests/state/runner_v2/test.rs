use ef_tests_state::runner_v2::{error::RunnerError, parser::parse_file, runner::run_tests};

#[tokio::main]
pub async fn main() -> Result<(), RunnerError> {
    let tests = parse_file().await;
    run_tests(tests).await?;
    Ok(())
}
