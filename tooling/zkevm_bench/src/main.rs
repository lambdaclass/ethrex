use clap::{Parser, Subcommand};

mod cache;
mod manifest;
mod report;

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
        Command::Run { .. } => todo!("Task 9"),
        Command::Compare { .. } => todo!("Task 5"),
        Command::Curate { .. } => todo!("Task 11"),
    }
}
