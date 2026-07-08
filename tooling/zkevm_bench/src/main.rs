use clap::{Parser, Subcommand};

mod cache;
mod compare;
mod curate;
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
}

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            workloads,
            filter,
            out,
            elf,
        } => run::run_bench(&workloads, filter.as_deref(), &out, elf.as_deref()),
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
    }
}
