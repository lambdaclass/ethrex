use clap::{Parser, Subcommand};

use crate::bench::run_and_measure;
use crate::constants::get_chain_config;
use crate::fetcher::{get_blockdata, or_latest};
use crate::run::{exec, prove};

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");
pub const BINARY_NAME: &str = env!("CARGO_BIN_NAME");

#[derive(Parser)]
#[command(name=BINARY_NAME, author, version=VERSION_STRING, about, long_about = None)]
pub struct EthrexReplayCLI {
    #[command(subcommand)]
    command: EthrexReplayCommand,
}

#[derive(Subcommand)]
enum SubcommandExecute {
    #[command(about = "Execute a single block.")]
    Block {
        #[arg(help = "Block to use. Uses the latest if not specified.")]
        block: Option<usize>,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: String,
        #[arg(
            long,
            default_value = "mainnet",
            env = "NETWORK",
            required = false,
            help = "Name or ChainID of the network to use"
        )]
        network: String,
        #[arg(long, required = false)]
        bench: bool,
    },
    #[command(name = "block-range", about = "Executes a range of blocks")]
    BlockRange {
        #[arg(help = "Starting block. (Inclusive)")]
        start: usize,
        #[arg(help = "Ending block. (Inclusive)")]
        end: usize,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: String,
        #[arg(
            long,
            default_value = "mainnet",
            env = "NETWORK",
            required = false,
            help = "Name or ChainID of the network to use"
        )]
        network: String,
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
                network,
                bench,
            } => {
                let chain_config = get_chain_config(&network)?;
                let block = or_latest(block, &rpc_url).await?;
                let cache = get_blockdata(&rpc_url, chain_config, block).await?;
                let body = async {
                    let gas_used = cache.blocks[0].header.gas_used as f64;
                    let res = exec(cache).await?;
                    Ok((gas_used, res))
                };
                let res = run_and_measure(bench, body).await?;
                println!("{}", res);
            }
            SubcommandExecute::BlockRange {
                start,
                end,
                network,
                ..
            } => {
                if start >= end {
                    return Err(eyre::Error::msg(
                        "starting point can't be greater than ending point",
                    ));
                }
                let _chain_config = get_chain_config(&network)?;
                todo!()
                // let cache = get_rangedata(&rpc_url, chain_config, start, end).await?;
                // let body = async {
                //     let gas_used = cache.blocks.iter().map(|b| b.header.gas_used as f64).sum();
                //     let res = exec(cache).await?;
                //     Ok((gas_used, res))
                // };
                // let res = run_and_measure(bench, body).await?;
                // println!("{}", res);
            }
        }
        Ok(())
    }
}

#[derive(Subcommand)]
enum SubcommandProve {
    #[command(about = "Proves a single block.")]
    Block {
        #[arg(help = "Block to use. Uses the latest if not specified.")]
        block: Option<usize>,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: String,
        #[arg(
            long,
            default_value = "mainnet",
            env = "NETWORK",
            required = false,
            help = "Name or ChainID of the network to use"
        )]
        network: String,
        #[arg(long, required = false)]
        bench: bool,
    },
    #[command(name = "block-range", about = "Proves a range of blocks")]
    BlockRange {
        #[arg(help = "Starting block. (Inclusive)")]
        start: usize,
        #[arg(help = "Ending block. (Inclusive)")]
        end: usize,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: String,
        #[arg(
            long,
            default_value = "mainnet",
            env = "NETWORK",
            required = false,
            help = "Name or ChainID of the network to use"
        )]
        network: String,
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
                network,
                bench,
            } => {
                let chain_config = get_chain_config(&network)?;
                let block = or_latest(block, &rpc_url).await?;
                let cache = get_blockdata(&rpc_url, chain_config, block).await?;
                let body = async {
                    let gas_used = cache.blocks[0].header.gas_used as f64;
                    let res = prove(cache).await?;
                    Ok((gas_used, res))
                };
                let res = run_and_measure(bench, body).await?;
                println!("{}", res);
            }
            SubcommandProve::BlockRange {
                start,
                end,
                network,
                ..
            } => {
                if start >= end {
                    return Err(eyre::Error::msg(
                        "starting point can't be greater than ending point",
                    ));
                }
                let _chain_config = get_chain_config(&network)?;
                todo!()
                // let cache = get_rangedata(&rpc_url, chain_config, start, end).await?;
                // let body = async {
                //     let gas_used = cache.blocks.iter().map(|b| b.header.gas_used as f64).sum();
                //     let res = prove(cache).await?;
                //     Ok((gas_used, res))
                // };
                // let res = run_and_measure(bench, body).await?;
                // println!("{}", res);
            }
        }
        Ok(())
    }
}

#[derive(Subcommand)]
enum EthrexReplayCommand {
    #[command(
        subcommand,
        about = "Execute blocks, ranges of blocks, or individual transactions."
    )]
    Execute(SubcommandExecute),
    #[command(
        subcommand,
        about = "Proves blocks, ranges of blocks, or individual transactions."
    )]
    Prove(SubcommandProve),
}

pub async fn start() -> eyre::Result<()> {
    let EthrexReplayCLI { command } = EthrexReplayCLI::parse();

    match command {
        EthrexReplayCommand::Execute(cmd) => cmd.run().await?,
        EthrexReplayCommand::Prove(cmd) => cmd.run().await?,
    };
    Ok(())
}
