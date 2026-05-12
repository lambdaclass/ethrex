use clap::{Parser, Subcommand};
use std::path::PathBuf;

use ethrex_evm::statetest::{StatetestArgs, runner};

mod run;

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Statetest(args) => runner::run(args),
        Command::Run(args) => run::execute(args),
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
    /// Execute raw EVM bytecode.
    Run(RunArgs),
}

/// Arguments for the `run` subcommand.
///
/// Flag names and defaults match geth's `cmd/evm` runner so this binary can
/// serve as a drop-in for differential testing with goevmlab.
///
/// Geth defaults (from main.go):
///   `--nomemory`      default: true  (memory disabled)
///   `--noreturndata`  default: true  (return data disabled)
///   `--nostack`       default: false (stack enabled)
///   `--gas`           default: 10_000_000_000
#[derive(clap::Args, Debug)]
pub struct RunArgs {
    /// Bytecode to execute (hex-encoded, positional).
    pub bytecode: Option<String>,

    /// Read bytecode from a file instead of the positional argument.
    /// Use `-` to read from stdin.
    #[arg(long = "codefile")]
    pub codefile: Option<PathBuf>,

    /// Enable EIP-3155 JSON trace output on stderr.
    #[arg(long = "json", default_value_t = false)]
    pub json: bool,

    /// Disable memory in trace output (geth default: true).
    #[arg(long = "nomemory", default_value_t = true)]
    pub nomemory: bool,

    /// Disable stack in trace output (geth default: false).
    #[arg(long = "nostack", default_value_t = false)]
    pub nostack: bool,

    /// Disable return data in trace output (geth default: true).
    #[arg(long = "noreturndata", default_value_t = true)]
    pub noreturndata: bool,

    /// Gas limit for execution (geth default: 10_000_000_000).
    #[arg(long = "gas")]
    pub gas: Option<u64>,

    /// Input calldata (hex-encoded, e.g. `0xdeadbeef`).
    #[arg(long = "input")]
    pub input: Option<String>,

    /// Sender address (hex, e.g. `0xabc...`).
    #[arg(long = "sender")]
    pub sender: Option<String>,

    /// Receiver address (hex, e.g. `0xabc...`).
    #[arg(long = "receiver")]
    pub receiver: Option<String>,

    /// Value to send with the call. Hex (`0x...`) or decimal.
    #[arg(long = "value")]
    pub value: Option<String>,

    /// Print a post-execution stats dump to stderr.
    #[arg(long = "statdump", default_value_t = false)]
    pub statdump: bool,

    /// Fork to execute under (e.g. `Prague`, `Cancun`, `Shanghai`).
    /// Default: `Prague`. ethrex-specific extension (no geth equivalent
    /// in `run`).
    #[arg(long = "ethrex-fork")]
    pub ethrex_fork: Option<String>,
}
