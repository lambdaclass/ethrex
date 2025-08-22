use std::{io::Write, time::SystemTime};

use clap::{Parser, Subcommand};
use ethrex_common::{
    H256,
    types::{AccountUpdate, Block, Receipt},
};
use ethrex_prover_lib::backends::Backend;
use ethrex_rpc::types::block_identifier::BlockTag;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};
use reqwest::Url;
use tracing::{error, info};

use crate::block_run_report::{BlockRunReport, ReplayerMode};
use crate::fetcher::{get_blockdata, get_rangedata};
use crate::plot_composition::plot;
use crate::run::{exec, prove, run_tx};
use crate::{bench::run_and_measure, fetcher::get_batchdata};
use ethrex_config::networks::Network;

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");
pub const BINARY_NAME: &str = env!("CARGO_BIN_NAME");

#[cfg(feature = "sp1")]
const BACKEND: Backend = Backend::SP1;
#[cfg(feature = "risc0")]
const BACKEND: Backend = Backend::RISC0;
#[cfg(not(any(feature = "risc0", feature = "sp1")))]
const BACKEND: Backend = Backend::Exec;

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
        rpc_url: Url,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
        #[arg(long, required = false)]
        bench: bool,
    },
    #[command(about = "Execute a single block.")]
    Blocks {
        #[arg(help = "List of blocks to execute.", num_args = 1.., value_delimiter = ',')]
        blocks: Vec<usize>,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: Url,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::mainnet(),
        )]
        network: Network,
        #[arg(long, required = false)]
        bench: bool,
        #[arg(long, required = false)]
        to_csv: bool,
    },
    #[command(about = "Executes a range of blocks")]
    BlockRange {
        #[arg(help = "Starting block. (Inclusive)")]
        start: usize,
        #[arg(help = "Ending block. (Inclusive)")]
        end: usize,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: Url,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
        #[arg(long, required = false)]
        bench: bool,
    },
    #[command(about = "Execute and return transaction info.", visible_alias = "tx")]
    Transaction {
        #[arg(help = "Transaction hash.")]
        tx_hash: H256,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: Url,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
        #[arg(long, required = false)]
        l2: bool,
    },
    #[command(about = "Execute an L2 batch.")]
    Batch {
        #[arg(help = "Batch number to use.")]
        batch: u64,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: Url,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
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
                let eth_client = EthClient::new(rpc_url.as_str())?;
                let block = or_latest(block)?;
                let cache = get_blockdata(eth_client, network.clone(), block).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    exec(BACKEND, cache).await?;
                    Ok(gas_used)
                };
                run_and_measure(future, bench).await?;
            }
            SubcommandExecute::Blocks {
                mut blocks,
                rpc_url,
                network,
                bench,
                to_csv,
            } => {
                blocks.sort();

                let eth_client = EthClient::new(rpc_url.as_str())?;

                #[cfg(feature = "sp1")]
                let replay_mode = ReplayerMode::ExecuteSP1;
                #[cfg(feature = "risc0")]
                let replay_mode = ReplayerMode::ExecuteRISC0;
                #[cfg(not(any(feature = "risc0", feature = "sp1")))]
                let replay_mode = ReplayerMode::Execute;

                for (i, block_number) in blocks.iter().enumerate() {
                    info!("Executing block {}/{}: {block_number}", i + 1, blocks.len());

                    let block = eth_client
                        .get_raw_block(BlockIdentifier::Number(*block_number as u64))
                        .await?;

                    let start = SystemTime::now();

                    let res = Box::pin(async {
                        SubcommandExecute::Block {
                            block: Some(*block_number),
                            rpc_url: rpc_url.clone(),
                            network: network.clone(),
                            bench,
                        }
                        .run()
                        .await
                    })
                    .await;

                    let elapsed = start.elapsed().unwrap_or_default();

                    let block_run_report = BlockRunReport::new_for(
                        block,
                        network.clone(),
                        res,
                        replay_mode.clone(),
                        elapsed,
                    );

                    if block_run_report.run_result.is_err() {
                        error!("{block_run_report}");
                    } else {
                        info!("{block_run_report}");
                    }

                    if to_csv {
                        let file_name = format!("ethrex_replay_{network}_{replay_mode}.csv",);

                        let mut file = std::fs::OpenOptions::new()
                            .append(true)
                            .create(true)
                            .open(file_name)?;

                        file.write_all(block_run_report.to_csv().as_bytes())?;

                        file.write_all(b"\n")?;

                        file.flush()?;
                    }
                }
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
                let eth_client = EthClient::new(rpc_url.as_str())?;
                let cache = get_rangedata(eth_client, network.clone(), start, end).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    exec(BACKEND, cache).await?;
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
                let eth_client = EthClient::new(rpc_url.as_str())?;

                // Get the block number of the transaction
                let tx = eth_client
                    .get_transaction_by_hash(tx_hash)
                    .await?
                    .ok_or(eyre::Error::msg("error fetching transaction"))?;
                let block_number = tx.block_number;

                let cache = get_blockdata(
                    eth_client,
                    network,
                    BlockIdentifier::Number(block_number.as_u64()),
                )
                .await?;

                let (receipt, transitions) = run_tx(cache, tx_hash, l2).await?;
                print_receipt(receipt);
                for transition in transitions {
                    print_transition(transition);
                }
            }
            SubcommandExecute::Batch {
                batch,
                rpc_url,
                network,
                bench,
            } => {
                // Note: I think this condition is not sufficient to determine if the network is an L2 network.
                // Take this into account if you are fixing this command.
                if let Network::PublicNetwork(_) = network {
                    return Err(eyre::Error::msg(
                        "Batch execution is only supported on L2 networks.",
                    ));
                }
                let chain_config = network.get_genesis()?.config;
                let rollup_client = EthClient::new(rpc_url.as_str())?;
                let cache = get_batchdata(rollup_client, chain_config, batch).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    exec(BACKEND, cache).await?;
                    Ok(gas_used)
                };
                run_and_measure(future, bench).await?;
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
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
        #[arg(long, required = false)]
        bench: bool,
    },
    #[command(about = "Execute a single block.")]
    Blocks {
        #[arg(help = "List of blocks to execute.", num_args = 1.., value_delimiter = ',')]
        blocks: Vec<usize>,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: Url,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::mainnet(),
        )]
        network: Network,
        #[arg(long, required = false)]
        bench: bool,
        #[arg(long, required = false)]
        to_csv: bool,
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
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
        #[arg(long, required = false)]
        bench: bool,
    },
    #[command(about = "Proves an L2 batch.")]
    Batch {
        #[arg(help = "Batch number to use.")]
        batch: u64,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: Url,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
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
                let eth_client = EthClient::new(&rpc_url)?;
                let block = or_latest(block)?;
                let cache = get_blockdata(eth_client, network.clone(), block).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    prove(BACKEND, cache).await?;
                    Ok(gas_used)
                };
                run_and_measure(future, bench).await?;
            }
            SubcommandProve::Blocks {
                mut blocks,
                rpc_url,
                network,
                bench,
                to_csv,
            } => {
                blocks.sort();

                let eth_client = EthClient::new(rpc_url.as_str())?;

                for (i, block_number) in blocks.iter().enumerate() {
                    info!("Proving block {}/{}: {block_number}", i + 1, blocks.len());

                    let block = eth_client
                        .get_raw_block(BlockIdentifier::Number(*block_number as u64))
                        .await?;

                    let start = SystemTime::now();

                    let res = Box::pin(async {
                        SubcommandProve::Block {
                            block: Some(*block_number),
                            rpc_url: rpc_url.as_str().to_string(),
                            network: network.clone(),
                            bench,
                        }
                        .run()
                        .await
                    })
                    .await;

                    let elapsed = start.elapsed().unwrap_or_default();

                    let block_run_report = BlockRunReport::new_for(
                        block,
                        network.clone(),
                        res,
                        ReplayerMode::ProveSP1, // TODO: Support RISC0
                        elapsed,
                    );

                    if block_run_report.run_result.is_err() {
                        error!("{block_run_report}");
                    } else {
                        info!("{block_run_report}");
                    }

                    if to_csv {
                        let file_name =
                            format!("ethrex_replay_{network}_{}.csv", ReplayerMode::ProveSP1);

                        let mut file = std::fs::OpenOptions::new()
                            .append(true)
                            .create(true)
                            .open(file_name)?;

                        file.write_all(block_run_report.to_csv().as_bytes())?;

                        file.write_all(b"\n")?;

                        file.flush()?;
                    }
                }
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
                let eth_client = EthClient::new(&rpc_url)?;
                let cache = get_rangedata(eth_client, network.clone(), start, end).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    prove(BACKEND, cache).await?;
                    Ok(gas_used)
                };
                run_and_measure(future, bench).await?;
            }
            SubcommandProve::Batch {
                batch,
                rpc_url,
                network,
                bench,
            } => {
                let chain_config = network.get_genesis()?.config;
                let eth_client = EthClient::new(rpc_url.as_str())?;
                let cache = get_batchdata(eth_client, chain_config, batch).await?;
                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    prove(BACKEND, cache).await?;
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
    #[command(about = "Plots the composition of a range of blocks.")]
    BlockComposition {
        #[arg(help = "Starting block. (Inclusive)")]
        start: usize,
        #[arg(help = "Ending block. (Inclusive)")]
        end: usize,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: String,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
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
            let eth_client = EthClient::new(&rpc_url)?;
            let cache = get_rangedata(eth_client, network, start, end).await?;
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
