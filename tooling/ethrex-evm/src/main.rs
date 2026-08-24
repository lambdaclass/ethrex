//! `ethrex-evm`: EVM tooling binary for ethrex, mirroring geth's `evm` tool.
//!
//! Currently exposes a single subcommand, `t8n`, a geth-compatible state
//! transition tool used by test fillers (execution-spec-tests) to produce
//! test fixtures through LEVM.

mod t8n;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ethrex-evm", version, about = "ethrex EVM tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a state transition and output the post-state, geth `evm t8n`
    /// compatible.
    #[command(after_help = t8n::fork::supported_forks_help())]
    T8n(t8n::T8nArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::T8n(args) => t8n::run(args),
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
