use clap::{Parser, Subcommand};
use std::path::PathBuf;

use ethrex_evm::statetest::{StatetestArgs, runner};

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Statetest(args) => runner::run(args),
        Command::Run(args) => run_stub(args),
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
    /// Execute raw EVM bytecode (Phase 5).
    Run(RunArgs),
}

/// Arguments for the `run` subcommand (Phase 5).
#[derive(clap::Args, Debug)]
pub struct RunArgs {
    /// Bytecode to execute (hex-encoded, positional).
    pub bytecode: Option<String>,

    /// Read bytecode from a file instead of the positional argument.
    #[arg(long = "codefile")]
    pub codefile: Option<PathBuf>,

    /// Enable EIP-3155 JSON trace output.
    #[arg(long = "json", default_value_t = false)]
    pub json: bool,

    /// Disable memory in trace output.
    #[arg(long = "nomemory", default_value_t = false)]
    pub nomemory: bool,

    /// Disable stack in trace output.
    #[arg(long = "nostack", default_value_t = false)]
    pub nostack: bool,

    /// Disable return data in trace output.
    #[arg(long = "noreturndata", default_value_t = false)]
    pub noreturndata: bool,

    /// Gas limit for execution.
    #[arg(long = "gas")]
    pub gas: Option<u64>,

    /// Input data (hex-encoded).
    #[arg(long = "input")]
    pub input: Option<String>,

    /// Sender address.
    #[arg(long = "sender")]
    pub sender: Option<String>,

    /// Receiver address.
    #[arg(long = "receiver")]
    pub receiver: Option<String>,

    /// Print a post-execution state dump.
    #[arg(long = "statdump", default_value_t = false)]
    pub statdump: bool,
}

fn run_stub(_args: RunArgs) -> eyre::Result<()> {
    eyre::bail!("run subcommand: not yet implemented in this PR; Phase 5")
}
