use std::fs;
use std::process;

use clap::{Parser, Subcommand};
#[cfg(feature = "jit-bench")]
use tokamak_bench::report::{jit_suite_to_json, jit_to_markdown};
use tokamak_bench::{
    regression::compare,
    report::{from_json, regression_to_json, to_json, to_markdown},
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
