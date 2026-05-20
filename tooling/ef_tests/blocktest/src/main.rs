use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use clap::Parser;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use regex::Regex;
use serde::Serialize;

use ef_tests_blockchain::test_runner::parse_and_execute;

/// Fast blockchain test runner for ethrex -- CLI binary with parallel execution.
#[derive(Parser, Debug)]
#[command(name = "ef_tests-blockfast")]
struct Cli {
    /// Path to a blockchain test directory or a single .json file.
    #[arg(short, long, value_name = "PATH")]
    path: PathBuf,

    /// Number of parallel worker threads.
    #[arg(short, long, default_value = "1")]
    workers: usize,

    /// Regex filter: only run tests whose file path matches this pattern.
    #[arg(long)]
    run: Option<String>,

    /// Output results as JSON array to stdout.
    #[arg(long)]
    json: bool,
}

/// Known tests to skip (same as SKIPPED_BASE in blockchain/tests/all.rs).
const SKIPPED: &[&str] = &[
    // Skip because they take too long to run, but they pass
    "static_Call50000_sha256",
    "CALLBlake2f_MaxRounds",
    "loopMul",
    // Skip because it tries to deserialize number > U256::MAX
    "ValueOverflowParis",
    // Skip because it's a "Create" Blob Transaction, which doesn't actually exist.
    "createBlobhashTx",
];

#[derive(Serialize)]
struct TestResult {
    name: String,
    pass: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    let start = Instant::now();

    // Discover all .json fixture files.
    eprintln!("Scanning for fixture files in {:?} ...", cli.path);
    let mut files = if cli.path.is_file() {
        vec![cli.path.clone()]
    } else {
        collect_json_files(&cli.path)
    };

    // Apply skip list: remove files whose name contains a skipped pattern.
    files.retain(|f| {
        let name = f.file_name().unwrap_or_default().to_string_lossy();
        !SKIPPED.iter().any(|s| name.contains(s))
    });

    // Apply --run regex filter on the full file path.
    if let Some(ref pattern) = cli.run {
        let re = Regex::new(pattern).unwrap_or_else(|e| {
            eprintln!("Invalid --run regex '{}': {}", pattern, e);
            std::process::exit(1);
        });
        files.retain(|f| re.is_match(&f.to_string_lossy()));
    }

    let parse_time = start.elapsed();
    eprintln!(
        "Found {} fixture files in {:.2}s",
        files.len(),
        parse_time.as_secs_f64(),
    );

    // Build rayon thread pool.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(cli.workers)
        .build()
        .expect("failed to build thread pool");

    let passing = AtomicUsize::new(0);
    let failing = AtomicUsize::new(0);

    let exec_start = Instant::now();

    let results: Vec<TestResult> = pool.install(|| {
        files
            .into_par_iter()
            .map(|file| {
                let name = file
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                // parse_and_execute handles:
                //   - JSON parsing
                //   - skipping pre-Merge forks
                //   - building store, executing blocks, verifying post-state
                //   - returning Ok(()) or Err with combined failure messages
                let result = parse_and_execute(&file, Some(SKIPPED), None);

                match result {
                    Ok(()) => {
                        passing.fetch_add(1, Ordering::Relaxed);
                        TestResult {
                            name,
                            pass: true,
                            error: None,
                        }
                    }
                    Err(e) => {
                        failing.fetch_add(1, Ordering::Relaxed);
                        TestResult {
                            name,
                            pass: false,
                            error: Some(e.to_string()),
                        }
                    }
                }
            })
            .collect()
    });

    let exec_time = exec_start.elapsed();
    let total = results.len();
    let passed = passing.load(Ordering::Relaxed);
    let failed = failing.load(Ordering::Relaxed);

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&results).unwrap()
        );
    } else {
        // Print failures to stderr.
        for r in &results {
            if !r.pass {
                eprintln!(
                    "FAIL: {} -- {}",
                    r.name,
                    r.error.as_deref().unwrap_or("unknown"),
                );
            }
        }
    }

    eprintln!(
        "\nTotal: {} | Passed: {} | Failed: {} | Time: {:.2}s (scan: {:.2}s, exec: {:.2}s)",
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

/// Recursively collect all .json files under a directory.
fn collect_json_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("Cannot read directory {:?}: {}", dir, e);
            return result;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            result.extend(collect_json_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "json") {
            result.push(path);
        }
    }
    result
}
