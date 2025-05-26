use clap::{Parser, Subcommand};

use crate::bench::run_and_measure;
use crate::fetcher::get_blockdata;
use crate::run::{exec, prove};

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name="Ethrex_replay_cli", author, version=VERSION_STRING, about, long_about = None)]
pub struct EthrexReplayCLI {
    #[command(subcommand)]
    command: EthrexReplayCommand,
}

#[derive(Subcommand)]
enum SubcommandExecute {
    #[clap(about = "Execute a single block.")]
    Block {
        block: usize,
        #[arg(long, default_value = "http://localhost:8545", env = "RPC_URL")]
        rpc_url: String,
        #[arg(long, required = false)]
        bench: bool,
    },
}

impl SubcommandExecute {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            SubcommandExecute::Block {
                block,
                rpc_url,
                bench,
            } => {
                let cache = get_blockdata(rpc_url, block).await?;
                let body = async {
                    let gas_used = cache.block.header.gas_used as f64;
                    let res = exec(cache).await?;
                    Ok((gas_used, res))
                };
                let res = run_and_measure(bench, body).await?;
                println!(
                    "executed. {} -> {}",
                    res.0.initial_state_hash, res.0.final_state_hash
                );
            }
        }
        Ok(())
    }
}

#[derive(Subcommand)]
enum SubcommandProve {
    #[clap(about = "Proves a single block.")]
    Block {
        block: usize,
        #[arg(long, default_value = "http://localhost:8545", env = "RPC_URL")]
        rpc_url: String,
        #[arg(long, required = false)]
        bench: bool,
    },
}

impl SubcommandProve {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            SubcommandProve::Block {
                block,
                rpc_url,
                bench,
            } => {
                let cache = get_blockdata(rpc_url, block).await?;
                let body = async {
                    let gas_used = cache.block.header.gas_used as f64;
                    let res = prove(cache).await?;
                    Ok((gas_used, res))
                };
                let res = run_and_measure(bench, body).await?;
                println!(
                    "proven. {} -> {}",
                    res.0.initial_state_hash, res.0.final_state_hash
                );
            }
        }
        Ok(())
    }
}

#[derive(Subcommand)]
enum EthrexReplayCommand {
    #[clap(
        subcommand,
        about = "Execute blocks, ranges of blocks, or individual transactions."
    )]
    Execute(SubcommandExecute),
    #[clap(
        subcommand,
        about = "Proves blocks, ranges of blocks, or individual transactions."
    )]
    Prove(SubcommandExecute),
}

pub async fn start() -> eyre::Result<()> {
    let EthrexReplayCLI { command } = EthrexReplayCLI::parse();

    match command {
        EthrexReplayCommand::Execute(cmd) => cmd.run().await?,
        EthrexReplayCommand::Prove(cmd) => cmd.run().await?,
    };
    Ok(())
}
