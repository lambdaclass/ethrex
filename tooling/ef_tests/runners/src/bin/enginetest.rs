//! Run `blockchain_test_engine` fixtures through ethrex's Engine API handlers,
//! in process: each payload goes to `engine_newPayloadV<n>` and each forkchoice
//! update to `engine_forkchoiceUpdatedV<n>`, at the versions the fixture names.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use ef_tests_engine::{EngineFixtureFile, RunOptions, run_fixture};
use ef_tests_runners::{
    FixtureResult, RunnerArgs, collect_json_files, install_bal_report, report, run_files,
    runner_error,
};

#[derive(Parser, Debug)]
#[command(
    name = "ethrex-enginetest",
    version,
    about = "Run blockchain_test_engine fixtures through ethrex's Engine API"
)]
struct Cli {
    #[command(flatten)]
    runner: RunnerArgs,
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
    let outcome = run_files(&files, cli.runner.workers, |file| {
        run_file(file, &cli.runner, &rt)
    });
    report(&outcome, cli.runner.json, started)
}

fn run_file(
    file: &Path,
    args: &RunnerArgs,
    rt: &tokio::runtime::Runtime,
) -> Result<Vec<FixtureResult>, String> {
    let fixtures: EngineFixtureFile = std::fs::read_to_string(file)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))?;
    let opts = RunOptions {
        bal_parallel_exec: !args.bal_sequential,
        precompile_cache: !args.no_precompile_cache,
        check_exception_names: false,
        ..RunOptions::from_env()
    };
    let mut results = Vec::new();
    for (name, fixture) in fixtures.iter() {
        let mut rejections = Vec::new();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            rt.block_on(run_fixture(name, fixture, &opts, &mut rejections))
        }));
        let outcome = match outcome {
            Ok(Err(failure)) if failure.is_skip() => continue,
            Ok(result) => Ok(result.map_err(|failure| failure.to_string())),
            Err(panic) => Err(panic),
        };
        results.push(FixtureResult::from_outcome(
            name.clone(),
            fixture.network.clone(),
            outcome,
            rejections.into_iter().map(Into::into).collect(),
        ));
    }
    Ok(results)
}
