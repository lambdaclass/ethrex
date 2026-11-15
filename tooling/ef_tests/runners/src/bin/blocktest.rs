//! Run `blockchain_test` fixtures through ethrex's block import.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use ef_tests_blockchain::test_runner::{RunOptions, SKIPPED_TESTS, run_ef_test, skip_reason};
use ef_tests_blockchain::types::TestUnit;
use ef_tests_runners::{
    FixtureResult, RunnerArgs, collect_json_files, install_bal_report, report, run_files,
    runner_error,
};

#[derive(Parser, Debug)]
#[command(
    name = "ethrex-blocktest",
    version,
    about = "Run blockchain_test fixtures through ethrex"
)]
struct Cli {
    #[command(flatten)]
    runner: RunnerArgs,

    /// Never deliver the fixture's access list, so every block runs without one and
    /// ethrex checks the list it builds against the header.
    #[arg(long)]
    bal_withhold: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    install_bal_report(&cli.runner);
    let started = Instant::now();
    let rt = tokio::runtime::Runtime::new().expect("failed to build the tokio runtime");
    let files = match collect_json_files(cli.runner.fixture_paths()) {
        Ok(files) => files,
        Err(errors) => {
            for e in errors {
                eprintln!("error: {e}");
            }
            return runner_error();
        }
    };
    let options = RunOptions {
        bal_parallel_exec: !cli.runner.bal_sequential,
        withhold_access_lists: cli.bal_withhold,
        precompile_cache: !cli.runner.no_precompile_cache,
        check_exception_names: false,
    };
    let outcome = run_files(&files, cli.runner.workers, |file| {
        run_file(file, &cli.runner, &options, &rt)
    });
    report(&outcome, cli.runner.json, started)
}

fn run_file(
    file: &Path,
    args: &RunnerArgs,
    options: &RunOptions,
    rt: &tokio::runtime::Runtime,
) -> Result<Vec<FixtureResult>, String> {
    let tests: HashMap<String, TestUnit> = std::fs::read_to_string(file)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))?;
    let mut tests: Vec<_> = tests.into_iter().collect();
    tests.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(tests
        .into_iter()
        .filter(|(name, test)| {
            args.selects(name) && skip_reason(name, test, Some(SKIPPED_TESTS)).is_none()
        })
        .map(|(name, test)| {
            let mut rejections = Vec::new();
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                rt.block_on(run_ef_test(&name, &test, false, options, &mut rejections))
            }));
            let rejections = rejections.into_iter().map(Into::into).collect();
            FixtureResult::from_outcome(name, format!("{:?}", test.network), outcome, rejections)
        })
        .collect())
}
