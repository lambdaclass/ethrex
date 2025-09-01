use std::{
    cmp::max,
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    time::SystemTime,
};

use clap::{Parser, Subcommand};
use ethrex_blockchain::{
    Blockchain, BlockchainType,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, Bytes, H256, U256,
    types::{
        AccountUpdate, Block, ELASTICITY_MULTIPLIER, GenesisAccount, Receipt,
        payload::PayloadBundle,
    },
};
use ethrex_prover_lib::backends::Backend;
use ethrex_rpc::types::block_identifier::BlockTag;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmEngine;
use reqwest::Url;
use tracing::{error, info};

#[cfg(feature = "l2")]
use ethrex_blockchain::validate_block;
#[cfg(feature = "l2")]
use ethrex_l2::sequencer::block_producer::build_payload;
#[cfg(feature = "l2")]
use ethrex_storage_rollup::StoreRollup;
#[cfg(feature = "l2")]
use ethrex_vm::BlockExecutionResult;
#[cfg(feature = "l2")]
use std::sync::Arc;

use crate::plot_composition::plot;
use crate::run::{exec, prove, run_tx};
use crate::{bench::run_and_measure, fetcher::get_batchdata};
use crate::{
    block_run_report::{BlockRunReport, ReplayerMode},
    cache::Cache,
};
use crate::{
    fetcher::{get_blockdata, get_rangedata},
    run,
};
use ethrex_config::networks::Network;

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

#[cfg(feature = "sp1")]
pub const BACKEND: Backend = Backend::SP1;
#[cfg(all(feature = "risc0", not(feature = "sp1")))]
pub const BACKEND: Backend = Backend::RISC0;
#[cfg(not(any(feature = "sp1", feature = "risc0")))]
pub const BACKEND: Backend = Backend::Exec;

#[cfg(feature = "sp1")]
pub const REPLAYER_MODE: ReplayerMode = ReplayerMode::ExecuteSP1;

#[cfg(all(feature = "risc0", not(feature = "sp1")))]
pub const REPLAYER_MODE: ReplayerMode = ReplayerMode::ExecuteRISC0;

#[cfg(not(any(feature = "sp1", feature = "risc0")))]
pub const REPLAYER_MODE: ReplayerMode = ReplayerMode::Execute;

#[derive(Parser)]
#[command(name="ethrex-replay", author, version=VERSION_STRING, about, long_about = None)]
pub struct EthrexReplayCLI {
    #[command(subcommand)]
    command: EthrexReplayCommand,
}

#[derive(Subcommand)]
pub enum SubcommandExecute {
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
                        REPLAYER_MODE,
                        elapsed,
                    );

                    if block_run_report.run_result.is_err() {
                        error!("{block_run_report}");
                    } else {
                        info!("{block_run_report}");
                    }

                    if to_csv {
                        let file_name = format!("ethrex_replay_{network}_{}.csv", REPLAYER_MODE);

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
pub enum SubcommandProve {
    #[command(about = "Proves a single block.")]
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
                let eth_client = EthClient::new(rpc_url.as_str())?;
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
pub enum SubcommandCache {
    #[command(about = "Cache a single block.")]
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
    },
    #[command(about = "Cache multiple blocks.")]
    Blocks {
        #[arg(help = "List of blocks to execute.", num_args = 1.., value_delimiter = ',')]
        blocks: Vec<u64>,
        #[arg(long, env = "RPC_URL", required = true)]
        rpc_url: Url,
        #[arg(
            long,
            help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
            value_parser = clap::value_parser!(Network),
            default_value_t = Network::default(),
        )]
        network: Network,
    },
    #[command(about = "Cache a range of blocks")]
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
    },
}

impl SubcommandCache {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            SubcommandCache::Block {
                block,
                rpc_url,
                network,
            } => {
                let eth_client = EthClient::new(rpc_url.as_ref())?;
                let block_identifier = or_latest(block)?;
                let _ = get_blockdata(eth_client, network.clone(), block_identifier).await?;
                if let Some(block_number) = block {
                    info!("Block {block_number} data cached successfully.");
                } else {
                    info!("Latest block data cached successfully.");
                }
            }
            SubcommandCache::Blocks {
                mut blocks,
                rpc_url,
                network,
            } => {
                blocks.sort();
                let eth_client = EthClient::new(rpc_url.as_ref())?;
                for block_number in blocks {
                    let _ = get_blockdata(
                        eth_client.clone(),
                        network.clone(),
                        BlockIdentifier::Number(block_number),
                    )
                    .await?;
                }
                info!("Blocks data cached successfully.");
            }
            SubcommandCache::BlockRange {
                start,
                end,
                rpc_url,
                network,
            } => {
                let eth_client = EthClient::new(rpc_url.as_ref())?;
                let _ = get_rangedata(eth_client, network, start, end).await?;
                info!("Block from {start} to {end} data cached successfully.");
            }
        }
        Ok(())
    }
}

#[derive(Subcommand)]
pub enum SubcommandCustom {
    #[command(about = "Custom block.")]
    Block {
        #[arg(
            long,
            default_value_t = false,
            value_name = "BOOLEAN",
            conflicts_with = "prove",
            help = "Replayer will execute blocks"
        )]
        execute: bool,
        #[arg(
            long,
            default_value_t = false,
            value_name = "BOOLEAN",
            conflicts_with = "execute",
            help = "Replayer will prove block executions"
        )]
        prove: bool,
    },
    #[command(about = "Custom batch of blocks.")]
    Batch {
        #[arg(long, help = "Number of blocks to include in the batch.")]
        n_blocks: u64,
        #[arg(
            long,
            default_value_t = false,
            value_name = "BOOLEAN",
            group = "replayer_mode",
            required = true,
            help = "Replayer will execute batches"
        )]
        execute: bool,
        #[arg(
            long,
            default_value_t = false,
            value_name = "BOOLEAN",
            group = "replayer_mode",
            required = true,
            help = "Replayer will prove batch executions"
        )]
        prove: bool,
    },
}

impl SubcommandCustom {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            SubcommandCustom::Block { execute, prove: _ } => {
                if execute {
                    println!("Executing custom block");
                } else {
                    println!("Proving custom block");
                }

                let network = Network::LocalDevnet;

                let genesis = network.get_genesis()?;

                let mut store = {
                    let store_inner = Store::new("./", EngineType::InMemory)?;
                    store_inner.add_initial_state(genesis.clone()).await?;
                    store_inner
                };

                #[cfg(feature = "l2")]
                let rollup_store = {
                    use ethrex_storage_rollup::EngineTypeRollup;

                    let rollup_store = StoreRollup::new("./", EngineTypeRollup::InMemory)
                        .expect("Failed to create StoreRollup");
                    rollup_store
                        .init()
                        .await
                        .expect("Failed to init rollup store");
                    rollup_store
                };

                #[cfg(not(feature = "l2"))]
                let mut blockchain =
                    Blockchain::new(EvmEngine::LEVM, store.clone(), BlockchainType::L1, false);
                #[cfg(feature = "l2")]
                let blockchain = Arc::new(Blockchain::new(
                    EvmEngine::LEVM,
                    store.clone(),
                    BlockchainType::L2,
                ));

                let genesis_hash = genesis.get_block().hash();

                #[cfg(not(feature = "l2"))]
                let block = produce_l1_block(
                    &mut blockchain,
                    &mut store,
                    genesis_hash,
                    genesis.timestamp + 1,
                )
                .await?;
                #[cfg(feature = "l2")]
                let block = produce_l2_block(
                    blockchain.clone(),
                    &mut store,
                    &rollup_store,
                    genesis_hash,
                    genesis.timestamp + 1,
                )
                .await?;

                let blocks = vec![block];

                let execution_witness = blockchain.generate_witness_for_blocks(&blocks).await?;

                // Make cache mutable for L2 fields
                #[cfg_attr(
                    not(feature = "l2"),
                    expect(unused_mut, reason = "used in cfg feature l2")
                )]
                let mut cache = Cache::new(blocks, execution_witness);

                #[cfg(feature = "l2")]
                {
                    use crate::cache::L2Fields;

                    cache.l2_fields = Some(L2Fields {
                        blob_commitment: [0_u8; 48],
                        blob_proof: [0_u8; 48],
                    });
                }

                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    if execute {
                        exec(BACKEND, cache).await?;
                    } else {
                        prove(BACKEND, cache).await?;
                    }
                    Ok(gas_used)
                };

                let elapsed = run_and_measure(future, false).await?;

                if execute {
                    println!("Successfully executed custom block in {elapsed} seconds.");
                } else {
                    println!("Successfully proved custom block in {elapsed} seconds.");
                }
            }
            SubcommandCustom::Batch {
                n_blocks,
                execute,
                prove,
            } => {
                if execute {
                    println!(
                        "Executing batch with {}",
                        if n_blocks == 1 {
                            "1 block".to_owned()
                        } else {
                            format!("{n_blocks} blocks")
                        }
                    );
                } else if prove {
                    println!(
                        "Proving batch with {}",
                        if n_blocks == 1 {
                            "1 block".to_owned()
                        } else {
                            format!("{n_blocks} blocks")
                        }
                    );
                }

                let network = Network::LocalDevnet;

                let genesis = network.get_genesis()?;

                let mut store = {
                    let store_inner = Store::new("./", EngineType::InMemory)?;
                    store_inner.add_initial_state(genesis.clone()).await?;
                    store_inner
                };

                #[cfg(feature = "l2")]
                let rollup_store = {
                    use ethrex_storage_rollup::EngineTypeRollup;

                    let rollup_store = StoreRollup::new("./", EngineTypeRollup::InMemory)
                        .expect("Failed to create StoreRollup");
                    rollup_store
                        .init()
                        .await
                        .expect("Failed to init rollup store");
                    rollup_store
                };

                #[cfg(not(feature = "l2"))]
                let mut blockchain =
                    Blockchain::new(EvmEngine::LEVM, store.clone(), BlockchainType::L1, false);
                #[cfg(feature = "l2")]
                let blockchain = Arc::new(Blockchain::new(
                    EvmEngine::LEVM,
                    store.clone(),
                    BlockchainType::L2,
                ));

                let mut blocks = Vec::new();
                let mut head_block_hash = genesis.get_block().hash();
                let initial_timestamp = genesis.get_block().header.timestamp;
                for i in 1..=max(1, n_blocks) {
                    #[cfg(not(feature = "l2"))]
                    let block = produce_l1_block(
                        &mut blockchain,
                        &mut store,
                        head_block_hash,
                        initial_timestamp + i,
                    )
                    .await?;
                    #[cfg(feature = "l2")]
                    let block = produce_l2_block(
                        blockchain.clone(),
                        &mut store,
                        &rollup_store,
                        head_block_hash,
                        initial_timestamp + i,
                    )
                    .await?;

                    head_block_hash = block.hash();

                    blocks.push(block);
                }

                let execution_witness = blockchain.generate_witness_for_blocks(&blocks).await?;

                println!(
                    "Successfully generated witness for {} blocks.",
                    blocks.len()
                );

                // Make cache mutable for L2 fields
                #[cfg_attr(
                    not(feature = "l2"),
                    expect(unused_mut, reason = "used in cfg feature l2")
                )]
                let mut cache = Cache::new(blocks, execution_witness);

                #[cfg(feature = "l2")]
                {
                    use crate::cache::L2Fields;

                    cache.l2_fields = Some(L2Fields {
                        blob_commitment: [0_u8; 48],
                        blob_proof: [0_u8; 48],
                    });
                }

                let future = async {
                    let gas_used = get_total_gas_used(&cache.blocks);
                    if execute {
                        run::exec(BACKEND, cache).await?;
                    } else {
                        run::prove(BACKEND, cache).await?;
                    }
                    Ok(gas_used)
                };

                let elapsed = run_and_measure(future, false).await?;

                if execute {
                    println!("Successfully executed batch in {elapsed} seconds.");
                } else if prove {
                    println!("Successfully proved batch in {elapsed} seconds.");
                }
            }
        }
        Ok(())
    }
}

#[cfg(not(feature = "l2"))]
pub async fn produce_l1_block(
    blockchain: &mut Blockchain,
    store: &mut Store,
    head_block_hash: H256,
    timestamp: u64,
) -> eyre::Result<Block> {
    let build_payload_args = BuildPayloadArgs {
        parent: head_block_hash,
        timestamp,
        fee_recipient: Address::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
    };

    let payload = create_payload(&build_payload_args, store)?;

    let payload_id = build_payload_args.id()?;

    store.add_payload(payload_id, payload).await?;

    let incompleted_payload_bundle = store
        .get_payload(payload_id)
        .await?
        .expect("Storage returned None for existing payload");

    let payload_build_result = blockchain
        .build_payload(incompleted_payload_bundle.block)
        .await?;

    let completed_payload_bundle = PayloadBundle {
        block: payload_build_result.payload,
        block_value: payload_build_result.block_value,
        blobs_bundle: payload_build_result.blobs_bundle,
        requests: payload_build_result.requests,
        completed: true,
    };

    store
        .update_payload(payload_id, completed_payload_bundle.clone())
        .await?;

    let final_payload = store
        .get_payload(payload_id)
        .await?
        .expect("Storage returned None for existing payload");

    blockchain.add_block(&final_payload.block).await?;

    let new_block_hash = final_payload.block.hash();

    apply_fork_choice(store, new_block_hash, new_block_hash, new_block_hash).await?;

    Ok(completed_payload_bundle.block)
}

#[cfg(feature = "l2")]
pub async fn produce_l2_block(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    rollup_store: &StoreRollup,
    head_block_hash: H256,
    timestamp: u64,
) -> eyre::Result<Block> {
    let build_payload_args = BuildPayloadArgs {
        parent: head_block_hash,
        timestamp,
        fee_recipient: Address::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
    };

    let payload = create_payload(&build_payload_args, store)?;

    let payload_build_result =
        build_payload(blockchain.clone(), payload, store, rollup_store).await?;

    let new_block = payload_build_result.payload;

    let chain_config = store.get_chain_config()?;

    validate_block(
        &new_block,
        &store
            .get_block_header_by_hash(new_block.header.parent_hash)?
            .ok_or(eyre::Error::msg("Parent block header not found"))?,
        &chain_config,
        build_payload_args.elasticity_multiplier,
    )?;

    let account_updates = payload_build_result.account_updates;

    let execution_result = BlockExecutionResult {
        receipts: payload_build_result.receipts,
        requests: Vec::new(),
    };

    let account_updates_list = store
        .apply_account_updates_batch(new_block.header.parent_hash, &account_updates)
        .await?
        .ok_or(eyre::Error::msg(
            "Failed to apply account updates: parent block not found",
        ))?;

    blockchain
        .store_block(&new_block, account_updates_list, execution_result)
        .await?;

    rollup_store
        .store_account_updates_by_block_number(new_block.header.number, account_updates)
        .await?;

    let new_block_hash = new_block.hash();

    apply_fork_choice(store, new_block_hash, new_block_hash, new_block_hash).await?;

    Ok(new_block)
}

#[derive(Parser)]
pub struct SubcommandGenerateGenesis {
    #[arg(
        long,
        help = "Name of the network or genesis file. Supported: mainnet, holesky, sepolia, hoodi. Default: mainnet",
        value_parser = clap::value_parser!(Network),
        default_value_t = Network::default(),
    )]
    network: Network,
    #[arg(long, help = "Number of accounts to generate.")]
    num_accounts: u64,
    #[arg(
        long,
        help = "Balance for each account.",
        default_value_t = 1_000_000_000_000_000_000_000_000
    )]
    balance: u128,
    #[arg(
        long,
        help = "Output path for the genesis file.",
        default_value = "genesis.json"
    )]
    genesis_out: PathBuf,
    #[arg(
        long,
        help = "Output path for the private keys file.",
        default_value = "keys.txt"
    )]
    keys_out: PathBuf,
}

use secp256k1::{PublicKey, Secp256k1};
use sha3::{Digest, Keccak256};
impl SubcommandGenerateGenesis {
    pub async fn run(self) -> eyre::Result<()> {
        println!(
            "Generating genesis file with {} accounts...",
            self.num_accounts
        );

        let secp = Secp256k1::new();
        let mut keys_file = File::create(&self.keys_out)?;
        let mut alloc = BTreeMap::new();
        let balance = U256::from(self.balance);

        for _ in 0..self.num_accounts {
            let (secret_key, public_key) = secp.generate_keypair(&mut rand::thread_rng());
            let address = public_key_to_address(&public_key);

            writeln!(keys_file, "{}", hex::encode(secret_key.secret_bytes()))?;

            alloc.insert(
                address,
                GenesisAccount {
                    code: Bytes::new(),
                    storage: HashMap::new(),
                    balance,
                    nonce: 0,
                },
            );
        }

        let mut genesis = self.network.get_genesis()?;
        genesis.alloc = alloc;

        let file = BufWriter::new(File::create(&self.genesis_out)?);
        serde_json::to_writer_pretty(file, &genesis)?;

        println!(
            "Successfully generated genesis file '{}' and keys file '{}'",
            self.genesis_out.display(),
            self.keys_out.display()
        );

        Ok(())
    }
}

fn public_key_to_address(public_key: &PublicKey) -> Address {
    let public_key = public_key.serialize_uncompressed();
    let hash = Keccak256::digest(&public_key[1..]);
    Address::from_slice(&hash[12..])
}

#[derive(Subcommand)]
pub enum EthrexReplayCommand {
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
    #[command(
        subcommand,
        about = "Store the state prior to the execution of the block"
    )]
    Cache(SubcommandCache),
    #[command(subcommand, about = "Custom block or batch")]
    Custom(SubcommandCustom),
    #[command(about = "Generate a custom genesis file.")]
    GenerateGenesis(SubcommandGenerateGenesis),
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
        EthrexReplayCommand::Cache(cmd) => cmd.run().await?,
        EthrexReplayCommand::Custom(cmd) => cmd.run().await?,
        EthrexReplayCommand::GenerateGenesis(cmd) => cmd.run().await?,
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
