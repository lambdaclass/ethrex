use clap::{ArgGroup, Parser, Subcommand};
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, H256,
    types::{
        AccountState, AccountUpdate, Block, BlockHeader, Receipt,
        block_execution_witness::GuestProgramState,
    },
};
use ethrex_prover_lib::backend::Backend;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::types::block_identifier::BlockTag;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};
use ethrex_storage::Store;
use ethrex_storage::{hash_address, store_db::in_memory::Store as InMemoryStore};
use ethrex_trie::InMemoryTrieDB;
use eyre::Ok;
use reqwest::Url;
use std::{
    io::Write,
    sync::{Arc, RwLock},
    time::{Instant, SystemTime},
};
use tracing::info;

use crate::bench::run_and_measure;
use crate::fetcher::{get_blockdata, get_rangedata};
use crate::plot_composition::plot;
use crate::run::{exec, prove, run_tx};
use crate::{
    block_run_report::{BlockRunReport, ReplayerMode},
    cache::Cache,
};
use ethrex_config::networks::{
    HOLESKY_CHAIN_ID, HOODI_CHAIN_ID, MAINNET_CHAIN_ID, Network, PublicNetwork, SEPOLIA_CHAIN_ID,
};

#[cfg(feature = "l2")]
use crate::fetcher::get_batchdata;

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

#[cfg(feature = "sp1")]
pub const BACKEND: Backend = Backend::SP1;
#[cfg(all(feature = "risc0", not(feature = "sp1")))]
pub const BACKEND: Backend = Backend::RISC0;
#[cfg(not(any(feature = "sp1", feature = "risc0")))]
pub const BACKEND: Backend = Backend::Exec;

#[derive(Parser)]
#[command(name="ethrex-replay", author, version=VERSION_STRING, about, long_about = None)]
pub struct EthrexReplayCLI {
    #[command(subcommand)]
    pub command: EthrexReplayCommand,
}

#[derive(Subcommand)]
pub enum EthrexReplayCommand {
    #[cfg(not(feature = "l2"))]
    #[command(about = "Replay a single block")]
    Block(BlockOptions),
    #[cfg(not(feature = "l2"))]
    #[command(about = "Replay multiple blocks")]
    Blocks(BlocksOptions),
    #[cfg(not(feature = "l2"))]
    #[command(about = "Replay a range of blocks")]
    BlockRange(BlockRangeOptions),
    #[cfg(not(feature = "l2"))]
    #[command(about = "Plots the composition of a range of blocks.")]
    BlockComposition {
        #[arg(help = "Starting block. (Inclusive)")]
        start: u64,
        #[arg(help = "Ending block. (Inclusive)")]
        end: u64,
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
    #[cfg(not(feature = "l2"))]
    #[command(
        subcommand,
        about = "Store the state prior to the execution of the block"
    )]
    Cache(CacheSubcommand),
    #[cfg(not(feature = "l2"))]
    #[command(about = "Replay a single transaction")]
    Transaction(TransactionOpts),
    #[cfg(feature = "l2")]
    #[command(subcommand, about = "L2 specific commands")]
    L2(L2Subcommand),
}

#[cfg(feature = "l2")]
#[derive(Subcommand)]
pub enum L2Subcommand {
    #[command(about = "Replay an L2 batch")]
    Batch(BatchOptions),
    #[command(about = "Replay an L2 block")]
    Block(BlockOptions),
    #[command(about = "Replay an L2 transaction")]
    Transaction(TransactionOpts),
}

#[cfg(not(feature = "l2"))]
#[derive(Parser)]
pub enum CacheSubcommand {
    #[command(about = "Cache a single block.")]
    Block(BlockOptions),
    #[command(about = "Cache multiple blocks.")]
    Blocks(BlocksOptions),
    #[command(about = "Cache a range of blocks")]
    BlockRange(BlockRangeOptions),
}

#[derive(Parser, Clone)]
#[clap(group = ArgGroup::new("replay_mode").required(true))]
#[clap(group = ArgGroup::new("data_source").required(true))]
pub struct EthrexReplayOptions {
    #[arg(long, group = "replay_mode")]
    pub execute: bool,
    #[arg(long, group = "replay_mode")]
    pub prove: bool,
    #[arg(long, group = "data_source")]
    pub rpc_url: Url,
    #[arg(long, group = "data_source")]
    pub cached: bool,
    #[arg(long)]
    pub no_backend: bool,
    #[arg(long, required = false)]
    pub bench: bool,
    #[arg(long, required = false)]
    pub to_csv: bool,
}

#[derive(Parser)]
pub struct BlockOptions {
    #[arg(long, help = "Block to use. Uses the latest if not specified.")]
    pub block: Option<u64>,
    #[command(flatten)]
    pub opts: EthrexReplayOptions,
}

#[cfg(not(feature = "l2"))]
#[derive(Parser)]
pub struct BlocksOptions {
    #[arg(long, help = "List of blocks to execute.", num_args = 1.., value_delimiter = ',')]
    blocks: Vec<u64>,
    #[command(flatten)]
    opts: EthrexReplayOptions,
}

#[cfg(not(feature = "l2"))]
#[derive(Parser)]
pub struct BlockRangeOptions {
    #[arg(long, help = "Starting block. (Inclusive)")]
    start: u64,
    #[arg(long, help = "Ending block. (Inclusive)")]
    end: u64,
    #[command(flatten)]
    opts: EthrexReplayOptions,
}

#[derive(Parser)]
pub struct TransactionOpts {
    #[arg(long, help = "Transaction hash.")]
    tx_hash: H256,
    #[arg(
        long,
        help = "Is this an L2 transaction?",
        default_value_t = false,
        required = false
    )]
    l2: bool,
    #[command(flatten)]
    opts: EthrexReplayOptions,
}

#[cfg(feature = "l2")]
#[derive(Parser)]
pub struct BatchOptions {
    #[arg(long, help = "Batch number to use.")]
    batch: u64,
    #[command(flatten)]
    opts: EthrexReplayOptions,
}

impl EthrexReplayCommand {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            #[cfg(not(feature = "l2"))]
            Self::Block(block_opts) => replay_block(block_opts).await?,
            #[cfg(not(feature = "l2"))]
            Self::Blocks(BlocksOptions { mut blocks, opts }) => {
                if opts.cached {
                    unimplemented!("cached mode is not implemented yet");
                }

                blocks.sort();

                for (i, block_number) in blocks.iter().enumerate() {
                    info!(
                        "{} block {}/{}: {block_number}",
                        if opts.execute { "Executing" } else { "Proving" },
                        i + 1,
                        blocks.len()
                    );

                    replay_block(BlockOptions {
                        block: Some(*block_number),
                        opts: opts.clone(),
                    })
                    .await?;
                }
            }
            #[cfg(not(feature = "l2"))]
            Self::BlockRange(BlockRangeOptions { start, end, opts }) => {
                if opts.cached {
                    unimplemented!("cached mode is not implemented yet");
                }

                if start >= end {
                    return Err(eyre::Error::msg(
                        "starting point can't be greater than ending point",
                    ));
                }

                for block in start..=end {
                    replay_block(BlockOptions {
                        block: Some(block),
                        opts: opts.clone(),
                    })
                    .await?;
                }
            }
            #[cfg(not(feature = "l2"))]
            Self::Cache(CacheSubcommand::Block(BlockOptions { block, opts })) => {
                let (eth_client, network) = setup(&opts, false).await?;

                let block_identifier = or_latest(block)?;

                get_blockdata(eth_client, network.clone(), block_identifier).await?;

                if let Some(block_number) = block {
                    info!("Block {block_number} data cached successfully.");
                } else {
                    info!("Latest block data cached successfully.");
                }
            }
            #[cfg(not(feature = "l2"))]
            Self::Cache(CacheSubcommand::Blocks(BlocksOptions { mut blocks, opts })) => {
                blocks.sort();

                let (eth_client, network) = setup(&opts, false).await?;

                for block_number in blocks {
                    get_blockdata(
                        eth_client.clone(),
                        network.clone(),
                        BlockIdentifier::Number(block_number),
                    )
                    .await?;
                }

                info!("Blocks data cached successfully.");
            }
            #[cfg(not(feature = "l2"))]
            Self::Cache(CacheSubcommand::BlockRange(BlockRangeOptions { start, end, opts })) => {
                let (eth_client, network) = setup(&opts, false).await?;

                get_rangedata(eth_client, network, start, end).await?;

                info!("Block from {start} to {end} data cached successfully.");
            }
            #[cfg(not(feature = "l2"))]
            Self::Transaction(opts) => replay_transaction(opts).await?,
            #[cfg(not(feature = "l2"))]
            Self::BlockComposition {
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
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Transaction(TransactionOpts {
                tx_hash,
                opts,
                l2: _,
            })) => {
                replay_transaction(TransactionOpts {
                    tx_hash,
                    opts,
                    l2: true,
                })
                .await?
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Batch(BatchOptions { batch, opts })) => {
                if opts.cached {
                    unimplemented!("cached mode is not implemented yet");
                }

                let (eth_client, network) = setup(&opts, true).await?;

                let cache = get_batchdata(eth_client, network, batch).await?;

                run_and_measure(replay(cache, &opts), opts.bench).await?;
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Block(block_opts)) => replay_block(block_opts).await?,
        }

        Ok(())
    }
}

async fn setup(opts: &EthrexReplayOptions, l2: bool) -> eyre::Result<(EthClient, Network)> {
    let eth_client = EthClient::new(opts.rpc_url.as_str())?;
    let chain_id = eth_client.get_chain_id().await?.as_u64();
    let network = network_from_chain_id(chain_id, l2);
    Ok((eth_client, network))
}

async fn replay(cache: Cache, opts: &EthrexReplayOptions) -> eyre::Result<f64> {
    let gas_used = get_total_gas_used(&cache.blocks);

    if opts.execute {
        exec(BACKEND, cache).await?;
    } else {
        prove(BACKEND, cache).await?;
    }

    Ok(gas_used)
}

async fn replay_no_backend(mut cache: Cache, opts: &EthrexReplayOptions) -> eyre::Result<f64> {
    let gas_used = get_total_gas_used(&cache.blocks);

    if opts.prove {
        panic!("Proving not enabled without backend");
    }

    let start = Instant::now();
    info!("Preparing Storage for execution without prover backend");

    // TODO: handle this better?
    cache.witness.nodes.retain(|v| v != &[0x80]);

    let guest_program = GuestProgramState::try_from(cache.witness.clone()).unwrap();

    let in_memory_store = InMemoryStore::new();
    // Set up internal state of in-memory store
    {
        let mut inner_store = in_memory_store.inner().unwrap();

        // Set up state trie nodes
        let all_nodes = &guest_program.nodes_hashed;
        let state_root_hash = guest_program.parent_block_header.state_root;

        let state_trie_nodes = InMemoryTrieDB::from_nodes(state_root_hash, all_nodes)?.inner;

        inner_store.state_trie_nodes = state_trie_nodes;

        // Set up storage trie nodes
        let addresses: Vec<Address> = cache
            .witness
            .keys
            .iter()
            .filter(|k| k.len() == Address::len_bytes())
            .map(|k| Address::from_slice(k))
            .collect();

        for address in &addresses {
            let hashed_address = hash_address(address);

            // Account state may not be in the state trie
            let account_state_rlp_opt = guest_program
                .state_trie
                .as_ref()
                .unwrap()
                .get(&hashed_address)
                .unwrap();

            let Some(account_state_rlp) = account_state_rlp_opt else {
                continue;
            };

            let storage_root = AccountState::decode(&account_state_rlp)?.storage_root;

            let in_memory_trie = match InMemoryTrieDB::from_nodes(storage_root, all_nodes) {
                std::result::Result::Ok(trie) => trie.inner,
                Err(_) => continue,
            };

            inner_store
                .storage_trie_nodes
                .insert(H256::from_slice(&hashed_address), in_memory_trie);
        }
    }

    // Set up store with preloaded database and the right chain config.
    let store = Store {
        engine: Arc::new(in_memory_store),
        chain_config: Arc::new(RwLock::new(cache.witness.chain_config)),
        latest_block_header: Arc::new(RwLock::new(BlockHeader::default())),
    };

    // Add codes to DB
    for (code_hash, code) in guest_program.codes_hashed.clone() {
        store
            .add_account_code(code_hash, code.into())
            .await
            .unwrap();
    }

    // Add block headers to DB
    for (_n, header) in guest_program.block_headers.clone() {
        store.add_block_header(header.hash(), header).await.unwrap();
    }

    let blockchain = Blockchain::default_with_store(store);

    info!("Storage preparation finished in {:.2?}", start.elapsed());

    for block in cache.blocks {
        info!("Starting to execute block {}", block.header.number);
        let start_time = Instant::now();
        blockchain.add_block(&block).await?;
        let duration = start_time.elapsed();
        info!("add_block execution time: {:.2?}", duration);
    }

    Ok(gas_used)
}

async fn replay_transaction(tx_opts: TransactionOpts) -> eyre::Result<()> {
    if tx_opts.opts.cached {
        unimplemented!("cached mode is not implemented yet");
    }

    let tx_hash = tx_opts.tx_hash;

    let l2 = tx_opts.l2;

    let (eth_client, network) = setup(&tx_opts.opts, l2).await?;

    // Get the block number of the transaction
    let tx = eth_client
        .get_transaction_by_hash(tx_hash)
        .await?
        .ok_or(eyre::Error::msg("error fetching transaction"))?;

    let cache = get_blockdata(
        eth_client,
        network,
        BlockIdentifier::Number(tx.block_number.as_u64()),
    )
    .await?;

    let (receipt, transitions) = run_tx(cache, tx_hash, l2).await?;

    print_receipt(receipt);

    for transition in transitions {
        print_transition(transition);
    }

    Ok(())
}

async fn replay_block(block_opts: BlockOptions) -> eyre::Result<()> {
    let opts = block_opts.opts;

    let block = block_opts.block;

    if opts.cached {
        unimplemented!("cached mode is not implemented yet");
    }

    let l2 = false;

    let (eth_client, network) = setup(&opts, l2).await?;

    #[cfg(feature = "l2")]
    if network != Network::LocalDevnetL2 {
        return Err(eyre::Error::msg(
            "L2 mode is only supported on LocalDevnetL2 network",
        ));
    }

    let cache = get_blockdata(eth_client, network.clone(), or_latest(block)?).await?;

    let block =
        cache.blocks.first().cloned().ok_or_else(|| {
            eyre::Error::msg("no block found in the cache, this should never happen")
        })?;

    let replayer_mode = replayer_mode(opts.execute, opts.no_backend)?;

    let start = SystemTime::now();

    let block_run_result = if opts.no_backend {
        //TODO: Is it possible that Instant is more accurate for measuring these kind of things?
        run_and_measure(replay_no_backend(cache, &opts), opts.bench).await
    } else {
        run_and_measure(replay(cache, &opts), opts.bench).await
    };

    let block_run_report = BlockRunReport::new_for(
        block,
        network.clone(),
        block_run_result,
        replayer_mode.clone(),
        start.elapsed()?,
    );

    block_run_report.log();

    if opts.to_csv {
        let file_name = format!("ethrex_replay_{network}_{replayer_mode}.csv");

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(file_name)?;

        file.write_all(block_run_report.to_csv().as_bytes())?;

        file.write_all(b"\n")?;

        file.flush()?;
    }

    Ok(())
}

fn network_from_chain_id(chain_id: u64, l2: bool) -> Network {
    match chain_id {
        MAINNET_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Mainnet),
        HOLESKY_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Holesky),
        HOODI_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Hoodi),
        SEPOLIA_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Sepolia),
        _ => {
            if l2 {
                Network::LocalDevnetL2
            } else {
                Network::LocalDevnet
            }
        }
    }
}

pub fn replayer_mode(execute: bool, no_backend: bool) -> eyre::Result<ReplayerMode> {
    if no_backend {
        #[cfg(any(feature = "sp1", feature = "risc0"))]
        return Err(eyre::Error::msg(
            "no-backend mode is not supported with SP1 or RISC0 features enabled",
        ));
        return Ok(ReplayerMode::ExecuteNoBackend);
    }
    if execute {
        #[cfg(feature = "sp1")]
        return Ok(ReplayerMode::ExecuteSP1);
        #[cfg(all(feature = "risc0", not(feature = "sp1")))]
        return Ok(ReplayerMode::ExecuteRISC0);
        #[cfg(not(any(feature = "sp1", feature = "risc0")))]
        return Ok(ReplayerMode::Execute);
    } else {
        #[cfg(feature = "sp1")]
        return Ok(ReplayerMode::ProveSP1);
        #[cfg(all(feature = "risc0", not(feature = "sp1")))]
        return Ok(ReplayerMode::ProveRISC0);
        #[cfg(not(any(feature = "sp1", feature = "risc0")))]
        return Err(eyre::Error::msg(
            "proving mode is not supported without SP1 or RISC0 features",
        ));
    }
}

fn get_total_gas_used(blocks: &[Block]) -> f64 {
    blocks.iter().map(|b| b.header.gas_used).sum::<u64>() as f64
}

fn or_latest(maybe_number: Option<u64>) -> eyre::Result<BlockIdentifier> {
    Ok(match maybe_number {
        Some(n) => BlockIdentifier::Number(n),
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
