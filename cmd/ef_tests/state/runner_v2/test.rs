use ef_tests_state::runner_v2::{parser::parse_file, runner::run_tests};

#[tokio::main]
pub async fn main() {
    let tests = parse_file().await;
    run_tests(tests).await;
}
