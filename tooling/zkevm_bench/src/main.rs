use clap::{Parser, Subcommand};

mod cache;
mod compare;
mod curate;
mod generate;
mod manifest;
mod micro;
mod report;
mod run;

#[derive(Parser)]
#[command(
    name = "ethrex-zkevm-bench",
    about = "Deterministic zkEVM execution benchmark (ziskemu AIR-cost)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Execute the workload set under ziskemu and write a JSON report.
    Run {
        #[arg(long, default_value = "fixtures/manifest.toml")]
        workloads: String,
        #[arg(long)]
        filter: Option<String>,
        #[arg(long, default_value = "report.json")]
        out: String,
        #[arg(long)]
        elf: Option<String>,
        /// Tier ceiling: "quick" (fastest subset), "medium" (default;
        /// quick + medium/untagged workloads), or "slow" (the full
        /// manifest, plus `--stress-dir` fixtures if given).
        #[arg(long, default_value = "medium")]
        mode: String,
        /// Directory of extra generated stress fixtures (`*.json`/`*.json.gz`,
        /// Cache-format, e.g. from `generate-stress`) to run as additional
        /// `stress` workloads. Only used in `--mode slow`.
        #[arg(long)]
        stress_dir: Option<String>,
    },
    /// Diff two reports and flag regressions.
    Compare {
        baseline: String,
        head: String,
        #[arg(long, default_value_t = 3.0)]
        threshold_pct: f64,
        #[arg(long)]
        out: Option<String>,
    },
    /// Curate real-block fixtures from a directory of Cache files.
    Curate {
        #[arg(long)]
        cache_dir: String,
        #[arg(long, default_value = "curation.json")]
        out: String,
        #[arg(long)]
        ziskemu: bool,
    },
    /// Generate stress fixtures from EEST `blockchain_tests` using ethrex's
    /// own execution machinery to produce the witness (no external eth-act
    /// tool, no zisk toolchain).
    GenerateStress {
        /// Directory to walk (recursively) for `*.json` EEST
        /// `blockchain_test` files.
        #[arg(long)]
        input_dir: String,
        /// Directory to write generated `*.json.gz` Cache-format fixtures
        /// into (created if missing).
        #[arg(long)]
        out_dir: String,
    },
}

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            workloads,
            filter,
            out,
            elf,
            mode,
            stress_dir,
        } => run::run_bench(
            &workloads,
            filter.as_deref(),
            &out,
            elf.as_deref(),
            &mode,
            stress_dir.as_deref(),
        ),
        Command::Compare {
            baseline,
            head,
            threshold_pct,
            out,
        } => {
            let code = compare::run_compare(&baseline, &head, threshold_pct, out.as_deref())?;
            std::process::exit(code);
        }
        Command::Curate {
            cache_dir,
            out,
            ziskemu,
        } => curate::run_curate(&cache_dir, &out, ziskemu),
        Command::GenerateStress { input_dir, out_dir } => {
            generate::run_generate_stress(&input_dir, &out_dir)
        }
    }
}
