use clap::{Parser, Subcommand};
use ethrex_common::types::{AccountUpdate, Receipt};

use crate::bench::run_and_measure;
use crate::constants::get_chain_config;
use crate::fetcher::{get_blockdata, get_rangedata, or_latest};
use crate::plot_composition::plot;
use crate::rpc::get_tx_block;
use crate::run::{exec, prove, run_tx};

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
    #[command(about = "Executes a range of blocks")]
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
    #[command(about = "Execute and return transaction info.", visible_alias = "tx")]
    Transaction {
        #[arg(help = "ID of the transaction")]
        tx: String,
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
                    exec(cache).await?;
                    Ok((gas_used, ()))
                };
                run_and_measure(bench, body).await?;
            }
            SubcommandExecute::BlockRange {
                start,
                end,
                rpc_url,
                network,
                bench,
            } => {
                if start >= end {
                    return Err(eyre::Error::msg(
                        "starting point can't be greater than ending point",
                    ));
                }
                let chain_config = get_chain_config(&network)?;
                let cache = get_rangedata(&rpc_url, chain_config, start, end).await?;
                let body = async {
                    let gas_used = cache.blocks.iter().map(|b| b.header.gas_used as f64).sum();
                    exec(cache).await?;
                    Ok((gas_used, ()))
                };
                run_and_measure(bench, body).await?;
            }
            SubcommandExecute::Transaction {
                tx,
                rpc_url,
                network,
            } => {
                let chain_config = get_chain_config(&network)?;
                let block_number = get_tx_block(&tx, &rpc_url).await?;
                let cache = get_blockdata(&rpc_url, chain_config, block_number).await?;
                let (receipt, transitions) = run_tx(cache, &tx).await?;
                print_receipt(receipt);
                for transition in transitions {
                    print_transition(transition);
                }
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
    #[command(about = "Proves a range of blocks")]
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
            long_help = "Name or ChainID of the network to use. The networks currently supported include holesky, sepolia, hoodi and mainnet."
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
                println!("{res}");
            }
            SubcommandProve::BlockRange {
                start,
                end,
                rpc_url,
                network,
                bench,
            } => {
                if start >= end {
                    return Err(eyre::Error::msg(
                        "starting point can't be greater than ending point",
                    ));
                }
                let chain_config = get_chain_config(&network)?;
                let cache = get_rangedata(&rpc_url, chain_config, start, end).await?;
                let body = async {
                    let gas_used = cache.blocks.iter().map(|b| b.header.gas_used as f64).sum();
                    let res = prove(cache).await?;
                    Ok((gas_used, res))
                };
                let res = run_and_measure(bench, body).await?;
                println!("{res}");
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
    #[command(about = "Proves blocks, ranges of blocks, or individual transactions.")]
    BlockComposition {
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
    },
}

pub async fn start() -> eyre::Result<()> {
    let EthrexReplayCLI { command } = EthrexReplayCLI::parse();

    match command {
        EthrexReplayCommand::Execute(cmd) => cmd.run().await?,
        EthrexReplayCommand::Prove(cmd) => cmd.run().await?,
        EthrexReplayCommand::BlockComposition {
            start,
            end,
            rpc_url,
            network,
        } => {
            if start >= end {
                return Err(eyre::Error::msg(
                    "starting point can't be greater than ending point",
                ));
            }
            let chain_config = get_chain_config(&network)?;
            let cache = get_rangedata(&rpc_url, chain_config, start, end).await?;
            plot(cache).await?;
        }
    };
    Ok(())
}

fn print_transition(update: AccountUpdate) {
    println!("Account {:x}", update.address);
    if update.removed {
        println!("  Account deleted.");
    }
    if let Some(info) = update.info {
        println!("  Updated AccountInfo:");
        println!("    New balance: {}", info.balance);
        println!("    New nonce: {}", info.nonce);
        println!("    New codehash: {:#x}", info.code_hash);
        if let Some(code) = update.code {
            println!("    New code: {}", hex::encode(code));
        }
    }
    if !update.added_storage.is_empty() {
        println!("  Updated Storage:");
    }
    for (key, value) in update.added_storage {
        println!("    {key:#x} = {value:#x}");
    }
}

fn print_receipt(receipt: Receipt) {
    if receipt.succeeded {
        println!("Transaction succeeded.")
    } else {
        println!("Transaction failed.")
    }
    println!("  Transaction type: {:?}", receipt.tx_type);
    println!("  Gas used: {}", receipt.cumulative_gas_used);
    if !receipt.logs.is_empty() {
        println!("  Logs: ");
    }
    for log in receipt.logs {
        let formatted_topics = log.topics.iter().map(|v| format!("{v:#x}"));
        println!(
            "    - {:#x} ({}) => {:#x}",
            log.address,
            formatted_topics.collect::<Vec<String>>().join(", "),
            log.data
        );
    }
}
