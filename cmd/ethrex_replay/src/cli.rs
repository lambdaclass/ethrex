use std::{cmp::max, fmt::Display, sync::Arc};

use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
use ethrex_blockchain::{
    Blockchain, BlockchainType,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H256,
    types::{AccountUpdate, Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Receipt},
};
use ethrex_prover_lib::backend::Backend;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};
use ethrex_rpc::{
    debug::execution_witness::RpcExecutionWitness, types::block_identifier::BlockTag,
};
use ethrex_storage::{EngineType, Store};
use reqwest::Url;
use tracing::info;

use crate::fetcher::{get_blockdata, get_rangedata};
use crate::plot_composition::plot;
use crate::report::Report;
use crate::run::{exec, prove, run_tx};
use crate::{cache::Cache, slack::try_send_report_to_slack};
use ethrex_config::networks::{
    HOLESKY_CHAIN_ID, HOODI_CHAIN_ID, MAINNET_CHAIN_ID, Network, PublicNetwork, SEPOLIA_CHAIN_ID,
};

#[cfg(feature = "l2")]
use crate::fetcher::get_batchdata;

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

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
    #[command(subcommand, about = "Replay a custom block or batch")]
    Custom(CustomSubcommand),
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
    #[command(subcommand, about = "Replay a custom L2 block or batch")]
    Custom(CustomSubcommand),
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
}

#[derive(Parser)]
pub enum CustomSubcommand {
    #[command(about = "Replay a single custom block")]
    Block(CustomBlockOptions),
    #[command(about = "Replay a single custom batch")]
    Batch(CustomBatchOptions),
}

#[derive(Parser, Clone)]
pub struct CommonOptions {
    #[arg(long, value_enum, help_heading = "Replay Options")]
    pub zkvm: Option<ZKVM>,
    #[arg(long, value_enum, default_value_t = Resource::default(), help_heading = "Replay Options")]
    pub resource: Resource,
    #[arg(long, value_enum, default_value_t = Action::default(), help_heading = "Replay Options")]
    pub action: Action,
}

#[derive(Parser, Clone)]
#[clap(group = ArgGroup::new("data_source").required(true))]
pub struct EthrexReplayOptions {
    #[command(flatten)]
    pub common: CommonOptions,
    #[arg(long, group = "data_source", help_heading = "Replay Options")]
    pub rpc_url: Url,
    #[arg(long, group = "data_source", help_heading = "Replay Options")]
    pub cached: bool,
    #[arg(long, required = false, help_heading = "Replay Options")]
    pub to_csv: bool,
    #[arg(long, default_value = "on", help_heading = "Replay Options")]
    pub cache_level: CacheLevel,
    #[arg(long, env = "SLACK_WEBHOOK_URL", help_heading = "Replay Options")]
    pub slack_webhook_url: Option<Url>,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum ZKVM {
    Jolt,
    Nexus,
    OpenVM,
    Pico,
    Risc0,
    SP1,
    Ziren,
    Zisk,
}

impl Display for ZKVM {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ZKVM::Jolt => "Jolt",
            ZKVM::Nexus => "Nexus",
            ZKVM::OpenVM => "OpenVM",
            ZKVM::Pico => "Pico",
            ZKVM::Risc0 => "RISC0",
            ZKVM::SP1 => "SP1",
            ZKVM::Ziren => "Ziren",
            ZKVM::Zisk => "ZisK",
        };
        write!(f, "{s}")
    }
}

#[derive(Clone, Debug, ValueEnum, Default)]
pub enum Resource {
    #[default]
    CPU,
    GPU,
}

impl Display for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Resource::CPU => "CPU",
            Resource::GPU => "GPU",
        };
        write!(f, "{s}")
    }
}

#[derive(Clone, Debug, ValueEnum, PartialEq, Eq, Default)]
pub enum Action {
    #[default]
    Execute,
    Prove,
}

impl Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Action::Execute => "Execute",
            Action::Prove => "Prove",
        };
        write!(f, "{s}")
    }
}

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq, Default)]
pub enum CacheLevel {
    Off,
    Failed,
    #[default]
    On,
}

#[derive(Parser)]
pub struct BlockOptions {
    #[arg(
        help = "Block to use. Uses the latest if not specified.",
        help_heading = "Command Options"
    )]
    pub block: Option<u64>,
    #[command(flatten)]
    pub opts: EthrexReplayOptions,
}

#[cfg(not(feature = "l2"))]
#[derive(Parser)]
#[command(group(ArgGroup::new("block_list").required(true).args(["blocks", "from"])))]
pub struct BlocksOptions {
    #[arg(help = "List of blocks to execute.", num_args = 1.., value_delimiter = ',', conflicts_with_all = ["from", "to"], help_heading = "Command Options")]
    blocks: Vec<u64>,
    #[arg(
        long,
        help = "Starting block. (Inclusive)",
        help_heading = "Command Options"
    )]
    from: Option<u64>,
    #[arg(
        long,
        help = "Ending block. (Inclusive)",
        requires = "from",
        help_heading = "Command Options"
    )]
    to: Option<u64>,
    #[command(flatten)]
    opts: EthrexReplayOptions,
}

#[derive(Parser)]
pub struct TransactionOpts {
    #[arg(help = "Transaction hash.", help_heading = "Command Options")]
    tx_hash: H256,
    #[command(flatten)]
    opts: EthrexReplayOptions,
}

#[cfg(feature = "l2")]
#[derive(Parser)]
pub struct BatchOptions {
    #[arg(long, help = "Batch number to use.", help_heading = "Command Options")]
    batch: u64,
    #[command(flatten)]
    opts: EthrexReplayOptions,
}

#[derive(Parser)]
pub struct CustomBlockOptions {
    #[command(flatten)]
    common: CommonOptions,
}

#[derive(Parser)]
pub struct CustomBatchOptions {
    #[arg(
        long,
        help = "Number of blocks to include in the batch.",
        help_heading = "Command Options"
    )]
    n_blocks: u64,
    #[command(flatten)]
    common: CommonOptions,
}

impl EthrexReplayCommand {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            #[cfg(not(feature = "l2"))]
            Self::Block(block_opts) => replay_block(block_opts).await?,
            #[cfg(not(feature = "l2"))]
            Self::Blocks(BlocksOptions {
                blocks,
                from,
                to,
                opts,
            }) => {
                if opts.cached {
                    unimplemented!("cached mode is not implemented yet");
                }

                let blocks = resolve_blocks(blocks, from, to, opts.rpc_url.clone()).await?;

                for (i, block_number) in blocks.iter().enumerate() {
                    info!(
                        "{} block {}/{}: {block_number}",
                        if opts.common.action == Action::Execute {
                            "Executing"
                        } else {
                            "Proving"
                        },
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
            Self::Cache(CacheSubcommand::Block(BlockOptions { block, opts })) => {
                let (eth_client, network) = setup(&opts).await?;

                let block_identifier = or_latest(block)?;

                get_blockdata(eth_client, network.clone(), block_identifier).await?;

                if let Some(block_number) = block {
                    info!("Block {block_number} data cached successfully.");
                } else {
                    info!("Latest block data cached successfully.");
                }
            }
            #[cfg(not(feature = "l2"))]
            Self::Cache(CacheSubcommand::Blocks(BlocksOptions {
                blocks,
                from,
                to,
                opts,
            })) => {
                let blocks = resolve_blocks(blocks, from, to, opts.rpc_url.clone()).await?;

                let (eth_client, network) = setup(&opts).await?;

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
            Self::Custom(CustomSubcommand::Block(CustomBlockOptions { common })) => {
                Box::pin(async move {
                    Self::Custom(CustomSubcommand::Batch(CustomBatchOptions {
                        n_blocks: 1,
                        common,
                    }))
                    .run()
                    .await
                })
                .await?;
            }
            #[cfg(not(feature = "l2"))]
            Self::Custom(CustomSubcommand::Batch(CustomBatchOptions { n_blocks, common })) => {
                let opts = EthrexReplayOptions {
                    rpc_url: Url::parse("http://localhost:8545")?,
                    cached: false,
                    to_csv: false,
                    cache_level: CacheLevel::default(),
                    common: common.clone(),
                    slack_webhook_url: None,
                };

                let elapsed = replay_custom_l1_blocks(max(1, n_blocks), opts).await?;

                match common.action {
                    Action::Execute => println!(
                        "Successfully executed {} in {elapsed:.2} seconds.",
                        if n_blocks > 1 { "batch" } else { "block" }
                    ),
                    Action::Prove => println!(
                        "Successfully proved {} in {elapsed:.2} seconds.",
                        if n_blocks > 1 { "batch" } else { "block" }
                    ),
                }
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
            Self::L2(L2Subcommand::Transaction(TransactionOpts { tx_hash, opts })) => {
                replay_transaction(TransactionOpts { tx_hash, opts }).await?
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Batch(BatchOptions { batch, opts })) => {
                if opts.cached {
                    unimplemented!("cached mode is not implemented yet");
                }

                let (eth_client, network) = setup(&opts).await?;

                let cache = get_batchdata(eth_client, network, batch).await?;

                run_and_measure(replay(cache, &opts), opts.bench).await?;
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Block(block_opts)) => replay_block(block_opts).await?,
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Custom(CustomSubcommand::Block(CustomBlockOptions {
                prove,
            }))) => {
                Box::pin(async move {
                    Self::L2(L2Subcommand::Custom(CustomSubcommand::Batch(
                        CustomBatchOptions { n_blocks: 1, prove },
                    )))
                    .run()
                    .await
                })
                .await?
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Custom(CustomSubcommand::Batch(CustomBatchOptions {
                n_blocks,
                prove,
            }))) => {
                let opts = EthrexReplayOptions {
                    execute: !prove,
                    prove,
                    rpc_url: Url::parse("http://localhost:8545")?,
                    cached: false,
                    bench: false,
                    to_csv: false,
                    cache_level: CacheLevel::default(),
                };

                let elapsed = replay_custom_l2_blocks(max(1, n_blocks), &opts).await?;

                if prove {
                    println!("Successfully proved L2 batch in {elapsed:.2} seconds.");
                } else {
                    println!("Successfully executed L2 batch in {elapsed:.2} seconds.");
                }
            }
        }

        Ok(())
    }
}

async fn setup(opts: &EthrexReplayOptions) -> eyre::Result<(EthClient, Network)> {
    let eth_client = EthClient::new(opts.rpc_url.as_str())?;
    let chain_id = eth_client.get_chain_id().await?.as_u64();
    let network = network_from_chain_id(chain_id);
    Ok((eth_client, network))
}

async fn replay_transaction(tx_opts: TransactionOpts) -> eyre::Result<()> {
    if tx_opts.opts.cached {
        unimplemented!("cached mode is not implemented yet");
    }

    let tx_hash = tx_opts.tx_hash;

    let (eth_client, network) = setup(&tx_opts.opts).await?;

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

    let (receipt, transitions) = run_tx(cache, tx_hash).await?;

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

    let (eth_client, network) = setup(&opts).await?;

    #[cfg(feature = "l2")]
    if network != Network::LocalDevnetL2 {
        return Err(eyre::Error::msg(
            "L2 mode is only supported on LocalDevnetL2 network",
        ));
    }

    let cache = get_blockdata(eth_client, network.clone(), or_latest(block)?).await?;

    // Always write the cache after fetching from RPC.
    // It will be deleted later if not needed.
    cache.write()?;

    let block =
        cache.blocks.first().cloned().ok_or_else(|| {
            eyre::Error::msg("no block found in the cache, this should never happen")
        })?;

    // Always execute
    let execution_result = exec(backend(&opts.common.zkvm)?, cache.clone()).await;

    let proving_result = if opts.common.action == Action::Prove {
        // Only prove if requested
        Some(prove(backend(&opts.common.zkvm)?, cache.clone()).await)
    } else {
        None
    };

    let report = Report::new_for(
        opts.common.zkvm,
        opts.common.resource,
        opts.common.action,
        block,
        network,
        execution_result,
        proving_result,
    );

    println!("{report}");

    try_send_report_to_slack(&report, opts.slack_webhook_url).await?;

    // Apply cache level rules
    match opts.cache_level {
        // Cache is already saved
        CacheLevel::On => {}
        // Only save the cache if the block run failed
        CacheLevel::Failed => {
            if report.execution_result.is_err() || report.proving_result.is_some_and(|r| r.is_err())
            {
                cache.delete()?;
            }
        }
        // Don't keep the cache
        CacheLevel::Off => cache.delete()?,
    }

    Ok(())
}

pub fn backend(zkvm: &Option<ZKVM>) -> eyre::Result<Backend> {
    match zkvm {
        Some(ZKVM::SP1) => {
            #[cfg(feature = "sp1")]
            return Ok(Backend::SP1);
            #[cfg(not(feature = "sp1"))]
            return Err(eyre::Error::msg("SP1 feature not enabled"));
        }
        Some(ZKVM::Risc0) => {
            #[cfg(feature = "risc0")]
            return Ok(Backend::RISC0);
            #[cfg(not(feature = "risc0"))]
            return Err(eyre::Error::msg("RISC0 feature not enabled"));
        }
        Some(_other) => Err(eyre::Error::msg(
            "Only SP1 and RISC0 backends are supported currently",
        )),
        None => Ok(Backend::Exec),
    }
}

pub(crate) fn network_from_chain_id(chain_id: u64) -> Network {
    match chain_id {
        MAINNET_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Mainnet),
        HOLESKY_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Holesky),
        HOODI_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Hoodi),
        SEPOLIA_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Sepolia),
        _ => {
            if cfg!(feature = "l2") {
                Network::LocalDevnetL2
            } else {
                Network::LocalDevnet
            }
        }
    }
}

fn or_latest(maybe_number: Option<u64>) -> eyre::Result<BlockIdentifier> {
    Ok(match maybe_number {
        Some(n) => BlockIdentifier::Number(n),
        None => BlockIdentifier::Tag(BlockTag::Latest),
    })
}

async fn resolve_blocks(
    mut blocks: Vec<u64>,
    from: Option<u64>,
    to: Option<u64>,
    rpc_url: Url,
) -> eyre::Result<Vec<u64>> {
    if let Some(start) = from {
        let end = to.unwrap_or(fetch_latest_block_number(rpc_url).await?);

        for block in start..=end {
            blocks.push(block);
        }
    } else {
        blocks.sort();
    }

    Ok(blocks)
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

pub async fn replay_custom_l1_blocks(
    n_blocks: u64,
    opts: EthrexReplayOptions,
) -> eyre::Result<Report> {
    let network = Network::LocalDevnet;

    let genesis = network.get_genesis()?;

    let mut store = {
        let store_inner = Store::new("./", EngineType::InMemory)?;
        store_inner.add_initial_state(genesis.clone()).await?;
        store_inner
    };

    let blockchain = Arc::new(Blockchain::new(store.clone(), BlockchainType::L1, false));

    let blocks = produce_l1_blocks(
        blockchain.clone(),
        &mut store,
        genesis.get_block().hash(),
        genesis.timestamp + 12,
        n_blocks,
    )
    .await?;

    let execution_witness = blockchain.generate_witness_for_blocks(&blocks).await?;

    let network = Network::try_from(execution_witness.chain_config.chain_id).map_err(|e| {
        eyre::Error::msg(format!("Failed to determine network from chain ID: {}", e))
    })?;

    let cache = Cache::new(
        blocks,
        RpcExecutionWitness::from(execution_witness),
        Some(network.clone()),
    );

    let execution_result = exec(backend(&opts.common.zkvm)?, cache.clone()).await;

    let proving_result = if opts.common.action == Action::Prove {
        // Only prove if requested
        Some(prove(backend(&opts.common.zkvm)?, cache.clone()).await)
    } else {
        None
    };

    let report = Report::new_for(
        opts.common.zkvm,
        opts.common.resource,
        opts.common.action,
        cache.blocks.first().cloned().ok_or_else(|| {
            eyre::Error::msg("no block found in the cache, this should never happen")
        })?,
        network,
        execution_result,
        proving_result,
    );

    Ok(report)
}

pub async fn produce_l1_blocks(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    head_block_hash: H256,
    initial_timestamp: u64,
    n_blocks: u64,
) -> eyre::Result<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut current_parent_hash = head_block_hash;
    let mut current_timestamp = initial_timestamp;

    for _ in 0..n_blocks {
        let block = produce_l1_block(
            blockchain.clone(),
            store,
            current_parent_hash,
            current_timestamp,
        )
        .await?;
        current_parent_hash = block.hash();
        current_timestamp += 12; // Assuming an average block time of 12 seconds
        blocks.push(block);
    }

    Ok(blocks)
}

pub async fn produce_l1_block(
    blockchain: Arc<Blockchain>,
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
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };

    let payload_id = build_payload_args.id()?;

    let payload = create_payload(&build_payload_args, store)?;

    blockchain
        .clone()
        .initiate_payload_build(payload, payload_id)
        .await;

    let PayloadBuildResult { payload: block, .. } = blockchain
        .get_payload(payload_id)
        .await
        .map_err(|err| match err {
            ethrex_blockchain::error::ChainError::UnknownPayload => {
                ethrex_rpc::RpcErr::UnknownPayload(format!(
                    "Payload with id {payload_id:#018x} not found",
                ))
            }
            err => ethrex_rpc::RpcErr::Internal(err.to_string()),
        })?;

    blockchain.add_block(&block).await?;

    let new_block_hash = block.hash();

    apply_fork_choice(store, new_block_hash, new_block_hash, new_block_hash).await?;

    Ok(block)
}

#[cfg(feature = "l2")]
use crate::cache::L2Fields;
#[cfg(feature = "l2")]
use ethrex_blockchain::validate_block;
#[cfg(feature = "l2")]
use ethrex_l2::sequencer::block_producer::build_payload;
#[cfg(feature = "l2")]
use ethrex_storage_rollup::StoreRollup;
#[cfg(feature = "l2")]
use ethrex_vm::BlockExecutionResult;

#[cfg(feature = "l2")]
pub async fn replay_custom_l2_blocks(
    n_blocks: u64,
    opts: &EthrexReplayOptions,
) -> eyre::Result<f64> {
    let network = Network::LocalDevnetL2;

    let genesis = network.get_genesis()?;

    let mut store = {
        let store_inner = Store::new("./", EngineType::InMemory)?;
        store_inner.add_initial_state(genesis.clone()).await?;
        store_inner
    };

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

    let blockchain = Arc::new(Blockchain::new(store.clone(), BlockchainType::L2, false));

    let genesis_hash = genesis.get_block().hash();

    let blocks = produce_custom_l2_blocks(
        blockchain.clone(),
        &mut store,
        &rollup_store,
        genesis_hash,
        genesis.timestamp + 1,
        n_blocks,
    )
    .await?;

    let execution_witness = blockchain.generate_witness_for_blocks(&blocks).await?;

    let network = Network::try_from(execution_witness.chain_config.chain_id).map_err(|e| {
        eyre::Error::msg(format!("Failed to determine network from chain ID: {}", e))
    })?;

    let mut cache = Cache::new(
        blocks,
        RpcExecutionWitness::from(execution_witness),
        Some(network),
    );

    cache.l2_fields = Some(L2Fields {
        blob_commitment: [0_u8; 48],
        blob_proof: [0_u8; 48],
    });

    let start = SystemTime::now();

    run_and_measure(replay(cache, opts), false).await?;

    let elapsed = start.elapsed()?.as_secs_f64();

    Ok(elapsed)
}

#[cfg(feature = "l2")]
pub async fn produce_custom_l2_blocks(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    rollup_store: &StoreRollup,
    head_block_hash: H256,
    initial_timestamp: u64,
    n_blocks: u64,
) -> eyre::Result<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut current_parent_hash = head_block_hash;
    let mut current_timestamp = initial_timestamp;

    for _ in 0..n_blocks {
        let block = produce_custom_l2_block(
            blockchain.clone(),
            store,
            rollup_store,
            current_parent_hash,
            current_timestamp,
        )
        .await?;
        current_parent_hash = block.hash();
        current_timestamp += 12; // Assuming an average block time of 12 seconds
        blocks.push(block);
    }

    Ok(blocks)
}

#[cfg(feature = "l2")]
pub async fn produce_custom_l2_block(
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

async fn fetch_latest_block_number(rpc_url: Url) -> eyre::Result<u64> {
    let eth_client = EthClient::new(rpc_url.as_str())?;

    let latest_block = eth_client.get_block_number().await?;

    Ok(latest_block.as_u64())
}
