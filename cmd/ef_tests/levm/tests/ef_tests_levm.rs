use clap::Parser;
use ef_tests_levm::{
    parser,
    runner::{self, EFTestRunnerOptions},
};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let opts = EFTestRunnerOptions::parse();
    let ef_tests = parser::parse_ef_tests(&opts)?;
    runner::run_ef_tests(ef_tests, &opts)?;
    Ok(())
}
