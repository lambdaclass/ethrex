use clap::Parser;
use ef_tests_state::{
    parser,
    runner::{self, EFTestRunnerOptions},
};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let opts = EFTestRunnerOptions::parse();
    let ef_tests = parser::parse_ef_tests(&opts)?;
    runner::run_ef_tests(ef_tests, &opts).await?;
    Ok(())
}
