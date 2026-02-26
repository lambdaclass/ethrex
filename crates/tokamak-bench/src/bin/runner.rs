use std::fs;
use std::process;

use clap::{Parser, Subcommand};
#[cfg(feature = "cross-client")]
use tokamak_bench::cross_client::{
    report as cross_report, runner as cross_runner, types as cross_types,
};
#[cfg(feature = "jit-bench")]
use tokamak_bench::report::{jit_suite_to_json, jit_to_markdown};
use tokamak_bench::{
    regression::{compare, compare_jit},
    report::{
        from_json, jit_regression_to_json, jit_regression_to_markdown, jit_suite_from_json,
        regression_to_json, to_json, to_markdown,
    },
    runner::{Scenario, default_scenarios, run_suite},
    types::Thresholds,
};

#[derive(Parser)]
#[command(name = "tokamak-bench", about = "Tokamak EVM benchmark runner")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run benchmark scenarios and output results as JSON
    Run {
        /// Comma-separated list of scenario names (default: all)
        #[arg(long)]
        scenarios: Option<String>,

        /// Number of runs per scenario
        #[arg(long, default_value = "10")]
        runs: u64,

        /// Number of warmup runs to discard before measurement
        #[arg(long, default_value = "2")]
        warmup: u64,

        /// Git commit hash for metadata
        #[arg(long, default_value = "unknown")]
        commit: String,

        /// Output JSON file path (default: stdout)
        #[arg(long)]
        output: Option<String>,
    },

    /// Compare baseline and current benchmark results
    Compare {
        /// Path to baseline JSON file
        #[arg(long)]
        baseline: String,

        /// Path to current JSON file
        #[arg(long)]
        current: String,

        /// Warning threshold percentage
        #[arg(long, default_value = "20.0")]
        threshold_warn: f64,

        /// Regression threshold percentage
        #[arg(long, default_value = "50.0")]
        threshold_regress: f64,

        /// Output JSON file path (default: stdout)
        #[arg(long)]
        output: Option<String>,
    },

    /// Generate a markdown report from a regression comparison JSON
    Report {
        /// Path to regression report JSON
        #[arg(long)]
        input: String,

        /// Output markdown file path (default: stdout)
        #[arg(long)]
        output: Option<String>,
    },

    /// Compare baseline and current JIT benchmark results for speedup regression
    JitCompare {
        /// Path to baseline JIT benchmark JSON file
        #[arg(long)]
        baseline: String,

        /// Path to current JIT benchmark JSON file
        #[arg(long)]
        current: String,

        /// Speedup drop threshold percentage (default: 20%)
        #[arg(long, default_value = "20.0")]
        threshold: f64,

        /// Output JSON file path (default: stdout as markdown)
        #[arg(long)]
        output: Option<String>,

        /// Output JSON instead of markdown
        #[arg(long)]
        json: bool,
    },

    /// Run cross-client benchmark comparison via eth_call (requires cross-client feature)
    #[cfg(feature = "cross-client")]
    CrossClient {
        /// Endpoints string: "geth=http://localhost:8546,reth=http://localhost:8547"
        #[arg(long)]
        endpoints: String,

        /// Comma-separated list of scenario names (default: all)
        #[arg(long)]
        scenarios: Option<String>,

        /// Number of runs per scenario
        #[arg(long, default_value = "10")]
        runs: u64,

        /// Number of warmup runs to discard before measurement
        #[arg(long, default_value = "2")]
        warmup: u64,

        /// Git commit hash for metadata
        #[arg(long, default_value = "unknown")]
        commit: String,

        /// Output file path (default: stdout)
        #[arg(long)]
        output: Option<String>,

        /// Output markdown instead of JSON
        #[arg(long)]
        markdown: bool,
    },

    /// Run JIT vs interpreter benchmark comparison (requires jit-bench feature)
    #[cfg(feature = "jit-bench")]
    JitBench {
        /// Comma-separated list of scenario names (default: all)
        #[arg(long)]
        scenarios: Option<String>,

        /// Number of runs per scenario
        #[arg(long, default_value = "10")]
        runs: u64,

        /// Number of warmup runs to discard before measurement
        #[arg(long, default_value = "2")]
        warmup: u64,

        /// Git commit hash for metadata
        #[arg(long, default_value = "unknown")]
        commit: String,

        /// Output file path (default: stdout as JSON)
        #[arg(long)]
        output: Option<String>,

        /// Output markdown instead of JSON
        #[arg(long)]
        markdown: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Run {
            scenarios,
            runs,
            warmup,
            commit,
            output,
        } => {
            let scenario_list: Vec<Scenario> = match &scenarios {
                Some(names) => {
                    let defaults = default_scenarios();
                    names
                        .split(',')
                        .filter_map(|name| {
                            let name = name.trim();
                            defaults.iter().find(|s| s.name == name).map(|s| Scenario {
                                name: s.name,
                                iterations: s.iterations,
                            })
                        })
                        .collect()
                }
                None => default_scenarios(),
            };

            if scenario_list.is_empty() {
                eprintln!("No valid scenarios selected");
                process::exit(1);
            }

            let suite = run_suite(&scenario_list, runs, warmup, &commit);
            let json = to_json(&suite);

            match output {
                Some(path) => {
                    fs::write(&path, &json).expect("Failed to write output");
                    eprintln!("Results written to {path}");
                }
                None => println!("{json}"),
            }
        }

        Command::Compare {
            baseline,
            current,
            threshold_warn,
            threshold_regress,
            output,
        } => {
            let baseline_json =
                fs::read_to_string(&baseline).expect("Failed to read baseline file");
            let current_json = fs::read_to_string(&current).expect("Failed to read current file");

            let baseline_suite = from_json(&baseline_json);
            let current_suite = from_json(&current_json);

            let thresholds = Thresholds {
                warning_percent: threshold_warn,
                regression_percent: threshold_regress,
            };

            let report = compare(&baseline_suite, &current_suite, &thresholds);
            let json = regression_to_json(&report);

            match output {
                Some(path) => {
                    fs::write(&path, &json).expect("Failed to write output");
                    eprintln!("Comparison written to {path}");
                }
                None => println!("{json}"),
            }

            // Exit with non-zero if regression detected
            if report.status == tokamak_bench::types::RegressionStatus::Regression {
                process::exit(1);
            }
        }

        Command::Report { input, output } => {
            let json = fs::read_to_string(&input).expect("Failed to read input file");
            let report = tokamak_bench::report::regression_from_json(&json);
            let md = to_markdown(&report);

            match output {
                Some(path) => {
                    fs::write(&path, &md).expect("Failed to write output");
                    eprintln!("Report written to {path}");
                }
                None => println!("{md}"),
            }
        }

        Command::JitCompare {
            baseline,
            current,
            threshold,
            output,
            json,
        } => {
            let baseline_json =
                fs::read_to_string(&baseline).expect("Failed to read baseline file");
            let current_json = fs::read_to_string(&current).expect("Failed to read current file");

            let baseline_suite = jit_suite_from_json(&baseline_json);
            let current_suite = jit_suite_from_json(&current_json);

            let report = compare_jit(&baseline_suite, &current_suite, threshold);

            let content = if json {
                jit_regression_to_json(&report)
            } else {
                jit_regression_to_markdown(&report)
            };

            match output {
                Some(path) => {
                    fs::write(&path, &content).expect("Failed to write output");
                    eprintln!("JIT comparison written to {path}");
                }
                None => println!("{content}"),
            }

            if report.status == tokamak_bench::types::RegressionStatus::Regression {
                process::exit(1);
            }
        }

        #[cfg(feature = "cross-client")]
        Command::CrossClient {
            endpoints,
            scenarios,
            runs,
            warmup,
            commit,
            output,
            markdown,
        } => {
            let client_endpoints = match cross_types::parse_endpoints(&endpoints) {
                Ok(eps) => eps,
                Err(e) => {
                    eprintln!("Invalid endpoints: {e}");
                    process::exit(1);
                }
            };

            let scenario_list: Vec<Scenario> = match &scenarios {
                Some(names) => {
                    let defaults = default_scenarios();
                    names
                        .split(',')
                        .filter_map(|name| {
                            let name = name.trim();
                            defaults.iter().find(|s| s.name == name).map(|s| Scenario {
                                name: s.name,
                                iterations: s.iterations,
                            })
                        })
                        .collect()
                }
                None => default_scenarios(),
            };

            if scenario_list.is_empty() {
                eprintln!("No valid scenarios selected");
                process::exit(1);
            }

            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            let suite = rt.block_on(cross_runner::run_cross_client_suite(
                &scenario_list,
                &client_endpoints,
                runs,
                warmup,
                &commit,
            ));

            let content = if markdown {
                cross_report::to_markdown(&suite)
            } else {
                cross_report::to_json(&suite)
            };

            match output {
                Some(path) => {
                    fs::write(&path, &content).expect("Failed to write output");
                    eprintln!("Cross-client results written to {path}");
                }
                None => println!("{content}"),
            }
        }

        #[cfg(feature = "jit-bench")]
        Command::JitBench {
            scenarios,
            runs,
            warmup,
            commit,
            output,
            markdown,
        } => {
            let scenario_list: Vec<Scenario> = match &scenarios {
                Some(names) => {
                    let defaults = default_scenarios();
                    names
                        .split(',')
                        .filter_map(|name| {
                            let name = name.trim();
                            defaults.iter().find(|s| s.name == name).map(|s| Scenario {
                                name: s.name,
                                iterations: s.iterations,
                            })
                        })
                        .collect()
                }
                None => default_scenarios(),
            };

            if scenario_list.is_empty() {
                eprintln!("No valid scenarios selected");
                process::exit(1);
            }

            let suite =
                tokamak_bench::jit_bench::run_jit_suite(&scenario_list, runs, warmup, &commit);

            let content = if markdown {
                jit_to_markdown(&suite)
            } else {
                jit_suite_to_json(&suite)
            };

            match output {
                Some(path) => {
                    fs::write(&path, &content).expect("Failed to write output");
                    eprintln!("JIT benchmark results written to {path}");
                }
                None => println!("{content}"),
            }
        }
    }
}
