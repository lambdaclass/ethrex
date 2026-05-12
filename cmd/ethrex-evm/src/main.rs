use clap::{Parser, Subcommand};

use ethrex_evm::statetest::{StatetestArgs, runner};

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Statetest(args) => runner::run(args),
    }
}

#[derive(Parser)]
#[command(name = "ethrex-evm", about = "EVM execution tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Execute EF-style state tests and stream EIP-3155 traces to stderr.
    Statetest(StatetestArgs),
}
