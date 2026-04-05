mod runner;
mod types;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use clap::Parser;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use regex::Regex;

use runner::TestResult;
use types::{Test, Tests};

/// Fast state test runner for ethrex LEVM -- no Store, no trie, no async.
#[derive(Parser, Debug)]
#[command(name = "ef_tests-statefast")]
struct Cli {
    /// Path to a state test directory or a single .json file.
    #[arg(short, long, value_name = "PATH")]
    path: PathBuf,

    /// Number of parallel worker threads.
    #[arg(short, long, default_value = "1")]
    workers: usize,

    /// Regex filter: only run tests whose name matches this pattern.
    #[arg(long)]
    run: Option<String>,

    /// Output results as JSON array to stdout.
    #[arg(long)]
    json: bool,

    /// Emit EIP-3155 per-opcode traces to stderr.
    #[arg(long)]
    trace: bool,
}

/// Tests to ignore (same set as state_v2).
const IGNORED_TESTS: &[&str] = &[
    "dynamicAccountOverwriteEmpty_Paris.json",
    "RevertInCreateInInitCreate2Paris.json",
    "RevertInCreateInInit_Paris.json",
    "create2collisionStorageParis.json",
    "InitCollisionParis.json",
    "InitCollision.json",
    "HighGasPrice.json",
    "HighGasPriceParis.json",
    "static_Call50000_sha256.json",
    "CALLBlake2f_MaxRounds.json",
    "loopMul.json",
    "ValueOverflow.json",
    "ValueOverflowParis.json",
    "contract_create.json",
];

fn main() {
    let cli = Cli::parse();

    let start = Instant::now();

    // Parse all test files.
    eprintln!("Parsing test files from {:?} ...", cli.path);
    let mut all_tests = if cli.path.is_file() {
        parse_file(&cli.path)
    } else {
        parse_dir(&cli.path)
    };

    // Apply --run regex filter.
    if let Some(ref pattern) = cli.run {
        let re = Regex::new(pattern).unwrap_or_else(|e| {
            eprintln!("Invalid --run regex '{}': {}", pattern, e);
            std::process::exit(1);
        });
        all_tests.retain(|t| re.is_match(&t.name));
    }

    let parse_time = start.elapsed();
    let total_cases: usize = all_tests.iter().map(|t| t.test_cases.len()).sum();
    eprintln!(
        "Parsed {} tests ({} sub-cases) in {:.2}s",
        all_tests.len(),
        total_cases,
        parse_time.as_secs_f64()
    );

    // Build rayon thread pool.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(cli.workers)
        .build()
        .expect("failed to build thread pool");

    let passing = AtomicUsize::new(0);
    let failing = AtomicUsize::new(0);
    let total_run = AtomicUsize::new(0);

    let exec_start = Instant::now();

    // Flatten all (test, case) pairs and run in parallel.
    let work_items: Vec<(&Test, usize)> = all_tests
        .iter()
        .flat_map(|test| (0..test.test_cases.len()).map(move |i| (test, i)))
        .collect();

    let trace = cli.trace;
    let results: Vec<TestResult> = pool.install(|| {
        work_items
            .into_par_iter()
            .map(|(test, case_idx)| {
                let tc = &test.test_cases[case_idx];
                let result =
                    runner::run_test_case(&test.name, &test.env, &test.pre, tc, trace);

                if result.pass {
                    passing.fetch_add(1, Ordering::Relaxed);
                } else {
                    failing.fetch_add(1, Ordering::Relaxed);
                }
                total_run.fetch_add(1, Ordering::Relaxed);
                result
            })
            .collect()
    });

    let exec_time = exec_start.elapsed();
    let total = total_run.load(Ordering::Relaxed);
    let passed = passing.load(Ordering::Relaxed);
    let failed = failing.load(Ordering::Relaxed);

    if cli.json {
        // JSON output mode.
        let json_results: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "pass": r.pass,
                    "fork": r.fork,
                    "stateRoot": r.state_root,
                    "error": r.error,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json_results).unwrap()
        );
    } else {
        // Print failures.
        for r in &results {
            if !r.pass {
                let err_msg = if r.error.is_empty() { "unknown" } else { &r.error };
                eprintln!("FAIL: {} -- {}", r.name, err_msg);
            }
        }
    }

    eprintln!(
        "\nTotal: {} | Passed: {} | Failed: {} | Time: {:.2}s (parse: {:.2}s, exec: {:.2}s)",
        total,
        passed,
        failed,
        start.elapsed().as_secs_f64(),
        parse_time.as_secs_f64(),
        exec_time.as_secs_f64(),
    );

    if failed > 0 {
        std::process::exit(1);
    }
}

// ---- File / directory parsing ----

fn parse_file(path: &PathBuf) -> Vec<Test> {
    let data = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("Cannot read {:?}: {}", path, e);
        std::process::exit(1);
    });
    let mut tests: Tests = serde_json::from_slice(&data).unwrap_or_else(|e| {
        eprintln!("JSON parse error in {:?}: {}", path, e);
        std::process::exit(1);
    });
    for test in tests.0.iter_mut() {
        test.path = path.clone();
    }
    tests.0
}

fn parse_dir(path: &PathBuf) -> Vec<Test> {
    let entries: Vec<_> = match std::fs::read_dir(path) {
        Ok(rd) => rd.flatten().collect(),
        Err(e) => {
            eprintln!("Cannot read directory {:?}: {}", path, e);
            std::process::exit(1);
        }
    };

    entries
        .into_par_iter()
        .flat_map(|entry| {
            let ft = entry.file_type().unwrap();
            if ft.is_dir() {
                parse_dir(&entry.path())
            } else if ft.is_file()
                && entry.path().extension().is_some_and(|ext| ext == "json")
            {
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                if IGNORED_TESTS.iter().any(|&skip| name_str == skip) {
                    return Vec::new();
                }
                parse_file(&entry.path())
            } else {
                Vec::new()
            }
        })
        .collect()
}
