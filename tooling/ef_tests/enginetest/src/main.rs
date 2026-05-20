#[allow(dead_code)]
mod runner;
#[allow(dead_code)]
mod types;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use clap::Parser;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use regex::Regex;
use runner::TestResult;
use types::EngineTestFile;

use ef_tests_blockchain::fork::Fork;

/// Engine test runner for ethrex -- runs blockchain_test_engine
/// fixtures through the real Engine API execution path.
#[derive(Parser, Debug)]
#[command(name = "ef_tests-enginetest")]
struct Cli {
    /// Path to a blockchain_tests_engine directory or a single
    /// .json file.
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
}

/// Tests to skip.
const SKIPPED: &[&str] = &[
    // Too slow but pass
    "static_Call50000_sha256",
    "CALLBlake2f_MaxRounds",
    "loopMul",
    // Deserialize error: number > U256::MAX
    "ValueOverflowParis",
    // Create Blob Transaction (invalid)
    "createBlobhashTx",
];

fn main() {
    let cli = Cli::parse();
    let start = Instant::now();

    eprintln!(
        "Scanning for engine test fixtures in {:?} ...",
        cli.path
    );

    let mut files = if cli.path.is_file() {
        vec![cli.path.clone()]
    } else {
        collect_json_files(&cli.path)
    };

    // Apply skip list
    files.retain(|f| {
        let name =
            f.file_name().unwrap_or_default().to_string_lossy();
        !SKIPPED.iter().any(|s| name.contains(s))
    });

    // Apply --run regex filter
    if let Some(ref pattern) = cli.run {
        let re = Regex::new(pattern).unwrap_or_else(|e| {
            eprintln!("Invalid --run regex '{pattern}': {e}");
            std::process::exit(1);
        });
        files.retain(|f| re.is_match(&f.to_string_lossy()));
    }

    let scan_time = start.elapsed();
    eprintln!(
        "Found {} fixture files in {:.2}s",
        files.len(),
        scan_time.as_secs_f64(),
    );

    // Build rayon thread pool
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
            .flat_map(|file| {
                let data = match std::fs::read(&file) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!(
                            "Cannot read {:?}: {e}",
                            file
                        );
                        return vec![TestResult {
                            name: file
                                .to_string_lossy()
                                .to_string(),
                            pass: false,
                            error: Some(format!(
                                "read error: {e}"
                            )),
                        }];
                    }
                };

                let test_map: EngineTestFile =
                    match serde_json::from_slice(&data) {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!(
                                "JSON parse error in {:?}: {e}",
                                file
                            );
                            return vec![TestResult {
                                name: file
                                    .to_string_lossy()
                                    .to_string(),
                                pass: false,
                                error: Some(format!(
                                    "parse error: {e}"
                                )),
                            }];
                        }
                    };

                let rt =
                    tokio::runtime::Runtime::new().unwrap();

                test_map
                    .into_iter()
                    .map(|(test_name, test)| {
                        // Skip pre-Merge forks
                        if test.network < Fork::Merge {
                            return TestResult {
                                name: test_name,
                                pass: true,
                                error: None,
                            };
                        }

                        // Skip tests whose names match the
                        // skip list
                        if SKIPPED.iter().any(|s| {
                            test_name.contains(s)
                        }) {
                            return TestResult {
                                name: test_name,
                                pass: true,
                                error: None,
                            };
                        }

                        let result = rt.block_on(
                            runner::run_engine_test(
                                &test_name,
                                &test,
                            ),
                        );

                        match result {
                            Ok(()) => {
                                passing.fetch_add(
                                    1,
                                    Ordering::Relaxed,
                                );
                                TestResult {
                                    name: test_name,
                                    pass: true,
                                    error: None,
                                }
                            }
                            Err(e) => {
                                failing.fetch_add(
                                    1,
                                    Ordering::Relaxed,
                                );
                                TestResult {
                                    name: test_name,
                                    pass: false,
                                    error: Some(e),
                                }
                            }
                        }
                    })
                    .collect::<Vec<_>>()
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
        "\nTotal: {} | Passed: {} | Failed: {} | \
         Time: {:.2}s (scan: {:.2}s, exec: {:.2}s)",
        total,
        passed,
        failed,
        start.elapsed().as_secs_f64(),
        scan_time.as_secs_f64(),
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
            eprintln!(
                "Cannot read directory {:?}: {e}",
                dir
            );
            return result;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            result.extend(collect_json_files(&path));
        } else if path
            .extension()
            .is_some_and(|ext| ext == "json")
        {
            result.push(path);
        }
    }
    result
}
