use clap::{Parser, Subcommand};
use ethrex_common::{
    H256,
    types::{AccountUpdate, Block, Receipt},
};
use ethrex_rpc::types::block_identifier::BlockTag;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};

use crate::bench::run_and_measure;
use crate::constants::get_chain_config;
use crate::fetcher::{get_blockdata, get_rangedata};
use crate::plot_composition::plot;
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
        #[arg(help = "Transaction hash.")]
        tx_hash: H256,
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
        l2: bool,
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
                let eth_client = EthClient::new(&rpc_url)?;
                let block = or_latest(block)?;
                let cache = get_blockdata(eth_client, chain_config, block).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    exec(cache).await?;
                    Ok(gas_used)
                };
                run_and_measure(future, bench).await?;
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
                let eth_client = EthClient::new(&rpc_url)?;
                let cache = get_rangedata(eth_client, chain_config, start, end).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    exec(cache).await?;
                    Ok(gas_used)
                };
                run_and_measure(future, bench).await?;
            }
            SubcommandExecute::Transaction {
                tx_hash,
                rpc_url,
                network,
                l2,
            } => {
                let chain_config = get_chain_config(&network)?;
                let eth_client = EthClient::new(&rpc_url)?;

                // Get the block number of the transaction
                let tx = eth_client
                    .get_transaction_by_hash(tx_hash)
                    .await?
                    .ok_or(eyre::Error::msg("error fetching transaction"))?;
                let block_number = tx.block_number;

                let cache = get_blockdata(
                    eth_client,
                    chain_config,
                    BlockIdentifier::Number(block_number.as_u64()),
                )
                .await?;

                let (receipt, transitions) = run_tx(cache, tx_hash, l2).await?;
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
                let eth_client = EthClient::new(&rpc_url)?;
                let block = or_latest(block)?;
                let cache = get_blockdata(eth_client, chain_config, block).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    prove(cache).await?;
                    Ok(gas_used)
                };
                run_and_measure(future, bench).await?;
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
                let eth_client = EthClient::new(&rpc_url)?;
                let cache = get_rangedata(eth_client, chain_config, start, end).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    prove(cache).await?;
                    Ok(gas_used)
                };
                run_and_measure(future, bench).await?;
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
            let eth_client = EthClient::new(&rpc_url)?;
            let cache = get_rangedata(eth_client, chain_config, start, end).await?;
            plot(cache).await?;
        }
    };
    Ok(())
}

fn get_total_gas_used(blocks: &[Block]) -> f64 {
    blocks.iter().map(|b| b.header.gas_used).sum::<u64>() as f64
}

fn or_latest(maybe_number: Option<usize>) -> eyre::Result<BlockIdentifier> {
    Ok(match maybe_number {
        Some(n) => BlockIdentifier::Number(n.try_into()?),
        None => BlockIdentifier::Tag(BlockTag::Latest),
    })
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
