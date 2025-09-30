use bytes::Bytes;
use std::{
    cmp::max,
    fmt::Display,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H256,
    types::{
        AccountState, AccountUpdate, Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL,
        ELASTICITY_MULTIPLIER, Receipt, block_execution_witness::GuestProgramState,
    },
};
use ethrex_prover_lib::backend::Backend;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_rpc::{
    EthClient,
    debug::execution_witness::{RpcExecutionWitness, execution_witness_from_rpc_chain_config},
    types::block_identifier::BlockIdentifier,
};
use ethrex_storage::{
    EngineType, Store, hash_address, store_db::in_memory::Store as InMemoryStore,
};
#[cfg(feature = "l2")]
use ethrex_storage_rollup::EngineTypeRollup;
use ethrex_trie::{
    InMemoryTrieDB, Node,
    node::{BranchNode, LeafNode},
};
use reqwest::Url;
#[cfg(feature = "l2")]
use std::path::Path;
use tracing::{debug, info};

#[cfg(feature = "l2")]
use crate::fetcher::get_batchdata;
use crate::helpers::get_referenced_hashes;
#[cfg(not(feature = "l2"))]
use crate::plot_composition::plot;
use crate::{cache::Cache, fetcher::get_blockdata, report::Report};
use crate::{
    run::{exec, prove, run_tx},
    slack::try_send_report_to_slack,
};
use ethrex_config::networks::{
    HOLESKY_CHAIN_ID, HOODI_CHAIN_ID, MAINNET_CHAIN_ID, Network, PublicNetwork, SEPOLIA_CHAIN_ID,
};

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
        rpc_url: Url,
    },
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
    #[arg(
        long,
        value_enum,
        help_heading = "Replay Options",
        conflicts_with = "no_zkvm"
    )]
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
    pub rpc_url: Option<Url>,
    #[arg(
        long,
        group = "data_source",
        help = "use cache as input instead of fetching from RPC",
        help_heading = "Replay Options",
        requires = "network",
        conflicts_with = "cache_level"
    )]
    pub cached: bool,
    #[arg(
        long,
        help = "Network to use for replay (i.e. mainnet, sepolia, hoodi). If not specified will fetch from RPC",
        value_enum,
        help_heading = "Replay Options"
    )]
    pub network: Option<Network>,
    #[arg(
        long,
        help = "Directory to store and load cache files",
        value_parser,
        default_value = "./replay_cache",
        help_heading = "Replay Options"
    )]
    pub cache_dir: PathBuf,
    #[arg(
        long,
        default_value = "on",
        help_heading = "Replay Options",
        help = "Criteria to save a cache when fetching from RPC",
        requires = "rpc_url"
    )]
    pub cache_level: CacheLevel,
    #[arg(long, env = "SLACK_WEBHOOK_URL", help_heading = "Replay Options")]
    pub slack_webhook_url: Option<Url>,
    #[arg(
        long,
        help = "Execute with `Blockchain::add_block`, without using zkvm as backend",
        help_heading = "Replay Options",
        conflicts_with = "zkvm"
    )]
    pub no_zkvm: bool,
    #[arg(
        long,
        short,
        help = "Enable verbose logging",
        help_heading = "Replay Options",
        required = false
    )]
    pub verbose: bool,
    // CAUTION
    // This flag is used to create a benchmark file that is used by our CI for
    // updating benchmarks from https://docs.ethrex.xyz/benchmarks/.
    // Do no remove it under any circumstances, unless you are refactoring how
    // we do benchmarks in CI.
    #[arg(
        long,
        help = "Generate a benchmark file named `bench_latest.json` with the latest execution rate in Mgas/s",
        help_heading = "CI Options",
        requires = "zkvm",
        default_value_t = false
    )]
    pub bench: bool,
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

#[derive(Parser, Clone)]
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
#[command(group(ArgGroup::new("block_list").required(true).multiple(true).args(["blocks", "from", "endless"])))]
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
    #[arg(
        long,
        help = "Run blocks endlessly, starting from the specified block or the latest if not specified.",
        help_heading = "Replay Options",
        conflicts_with_all = ["blocks", "to"]
    )]
    pub endless: bool,
    #[arg(
        long,
        help = "Only fetch Ethereum proofs blocks (i.e., no L2 blocks).",
        help_heading = "Replay Options",
        conflicts_with = "blocks"
    )]
    pub only_eth_proofs_blocks: bool,
    #[command(flatten)]
    opts: EthrexReplayOptions,
}

#[derive(Parser)]
pub struct TransactionOpts {
    #[arg(help = "Transaction hash.", help_heading = "Command Options")]
    tx_hash: H256,
    #[arg(
        long,
        help = "Block number containing the transaction. Necessary in cached mode.",
        help_heading = "Command Options"
    )]
    pub block_number: Option<u64>,
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
            Self::Block(block_opts) => replay_block(block_opts.clone()).await?,
            #[cfg(not(feature = "l2"))]
            Self::Blocks(BlocksOptions {
                mut blocks,
                from,
                to,
                endless,
                only_eth_proofs_blocks,
                opts,
            }) => {
                // Case ethrex-replay blocks n,...,m
                if !blocks.is_empty() {
                    blocks.sort();

                    for block in blocks.clone() {
                        info!(
                            "{} block: {block}",
                            if opts.common.action == Action::Execute {
                                "Executing"
                            } else {
                                "Proving"
                            }
                        );

                        Box::pin(async {
                            Self::Block(BlockOptions {
                                block: Some(block),
                                opts: opts.clone(),
                            })
                            .run()
                            .await
                        })
                        .await?;
                    }

                    return Ok(());
                }

                if opts.cached {
                    if from.is_none() || to.is_none() {
                        return Err(eyre::Error::msg(
                            "`from` and `to` must be specified when using cached mode",
                        ));
                    }
                }

                // It will only be used in case from or to weren't specified or in endless mode. We can unwrap as cached mode won't reach those places.
                let maybe_rpc = opts.rpc_url.as_ref();

                let from = match from {
                    // Case --from is set
                    // * --endless and --to cannot be set together (constraint by clap).
                    // * If --endless is set, we start from --from and keep checking for new blocks
                    // * If --to is set, we run from --from to --to and stop
                    Some(from) => from,
                    // Case --from is not set
                    // * If we reach this point, --endless must be set (constraint by clap)
                    None => {
                        fetch_latest_block_number(maybe_rpc.unwrap(), only_eth_proofs_blocks)
                            .await?
                    }
                };

                let to = match to {
                    // Case --to is set
                    // * If we reach this point, --from must be set and --endless is not set (constraint by clap)
                    Some(to) => to,
                    // Case --to is not set
                    // * If we reach this point, --from or --endless must be set (constraint by clap)
                    None => {
                        fetch_latest_block_number(maybe_rpc.unwrap(), only_eth_proofs_blocks)
                            .await?
                    }
                };

                if from > to {
                    return Err(eyre::Error::msg(
                        "starting point can't be greater than ending point",
                    ));
                }

                let mut block_to_replay = from;
                let mut last_block_to_replay = to;

                while block_to_replay <= last_block_to_replay {
                    if only_eth_proofs_blocks && block_to_replay % 100 != 0 {
                        block_to_replay += 1;

                        // Case --endless is set, we want to update the `to` so
                        // we can keep checking for new blocks
                        if endless && block_to_replay > last_block_to_replay {
                            last_block_to_replay = fetch_latest_block_number(
                                maybe_rpc.unwrap(),
                                only_eth_proofs_blocks,
                            )
                            .await?;

                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }

                        continue;
                    }

                    Box::pin(async {
                        Self::Block(BlockOptions {
                            block: Some(block_to_replay),
                            opts: opts.clone(),
                        })
                        .run()
                        .await
                    })
                    .await?;

                    block_to_replay += 1;

                    // Case --endless is set, we want to update the `to` so
                    // we can keep checking for new blocks
                    while endless && block_to_replay > last_block_to_replay {
                        last_block_to_replay =
                            fetch_latest_block_number(maybe_rpc.unwrap(), only_eth_proofs_blocks)
                                .await?;

                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
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
                    rpc_url: Some(Url::parse("http://localhost:8545")?),
                    cached: false,
                    no_zkvm: false,
                    cache_level: CacheLevel::default(),
                    common,
                    slack_webhook_url: None,
                    verbose: false,
                    bench: false,
                    cache_dir: PathBuf::from("./replay_cache"),
                    network: None,
                };

                let report = replay_custom_l1_blocks(max(1, n_blocks), opts).await?;

                println!("{report}");
            }
            #[cfg(not(feature = "l2"))]
            Self::Transaction(opts) => replay_transaction(opts).await?,
            #[cfg(not(feature = "l2"))]
            Self::BlockComposition {
                start,
                end,
                rpc_url,
            } => {
                if start >= end {
                    return Err(eyre::Error::msg(
                        "starting point can't be greater than ending point",
                    ));
                }

                let eth_client = EthClient::new(rpc_url.as_str())?;

                info!(
                    "Fetching blocks from RPC: {start} to {end} ({} blocks)",
                    end - start + 1
                );
                let mut blocks = vec![];
                for block_number in start..=end {
                    debug!("Fetching block {block_number}");
                    let rpc_block = eth_client
                        .get_block_by_number(BlockIdentifier::Number(block_number), true)
                        .await?;

                    let block = rpc_block
                        .try_into()
                        .map_err(|e| eyre::eyre!("Failed to convert rpc block to block: {}", e))?;

                    blocks.push(block);
                }

                plot(&blocks).await?;
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Transaction(TransactionOpts {
                tx_hash,
                opts,
                block_number,
            })) => {
                replay_transaction(TransactionOpts {
                    tx_hash,
                    opts,
                    block_number,
                })
                .await?
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Batch(BatchOptions { batch, opts })) => {
                if opts.cached {
                    unimplemented!("cached mode is not implemented yet");
                }

                let (eth_client, network) = setup_rpc(&opts).await?;

                let cache = get_batchdata(eth_client, network, batch, opts.cache_dir).await?;

                let backend = backend(&opts.common.zkvm)?;

                let execution_result = exec(backend, cache.clone()).await;

                let proving_result = match opts.common.action {
                    Action::Execute => None,
                    Action::Prove => Some(prove(backend, cache).await),
                };

                println!("Batch {batch} execution result: {execution_result:?}");

                if let Some(proving_result) = proving_result {
                    println!("Batch {batch} proving result: {proving_result:?}");
                }
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Block(block_opts)) => replay_block(block_opts).await?,
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Custom(CustomSubcommand::Block(CustomBlockOptions {
                common,
            }))) => {
                Box::pin(async move {
                    Self::L2(L2Subcommand::Custom(CustomSubcommand::Batch(
                        CustomBatchOptions {
                            n_blocks: 1,
                            common,
                        },
                    )))
                    .run()
                    .await
                })
                .await?
            }
            #[cfg(feature = "l2")]
            Self::L2(L2Subcommand::Custom(CustomSubcommand::Batch(CustomBatchOptions {
                n_blocks,
                common,
            }))) => {
                let opts = EthrexReplayOptions {
                    common,
                    rpc_url: Some(Url::parse("http://localhost:8545")?),
                    cached: false,
                    no_zkvm: false,
                    cache_level: CacheLevel::default(),
                    slack_webhook_url: None,
                    bench: false,
                    cache_dir: PathBuf::from("./replay_cache"),
                    verbose: false,
                    network: None,
                };

                let report = replay_custom_l2_blocks(max(1, n_blocks), opts).await?;

                println!("{report}");
            }
        }

        Ok(())
    }
}

pub async fn setup_rpc(opts: &EthrexReplayOptions) -> eyre::Result<(EthClient, Network)> {
    let eth_client = EthClient::new(opts.rpc_url.as_ref().unwrap().as_str())?;
    let chain_id = eth_client.get_chain_id().await?.as_u64();
    let network = network_from_chain_id(chain_id);
    Ok((eth_client, network))
}

async fn replay_no_zkvm(cache: Cache, opts: &EthrexReplayOptions) -> eyre::Result<Duration> {
    let b = backend(&opts.common.zkvm)?;
    if !matches!(b, Backend::Exec) {
        eyre::bail!("Tried to execute without zkVM but backend was set to {b:?}");
    }
    if opts.common.action == Action::Prove {
        eyre::bail!("Proving not enabled without backend");
    }
    if cache.blocks.len() > 1 {
        eyre::bail!("Cache for L1 witness should contain only one block.");
    }

    let start = Instant::now();
    info!("Preparing Storage for execution without zkVM");

    let chain_config = cache.get_chain_config()?;
    let block = cache.blocks[0].clone();

    let witness = execution_witness_from_rpc_chain_config(
        cache.witness.clone(),
        chain_config,
        cache.get_first_block_number()?,
    )?;
    let network = &cache.network;

    let guest_program = GuestProgramState::try_from(witness.clone())?;

    // This will contain all code hashes with the corresponding bytecode
    // For the code hashes that we don't have we'll fill it with <CodeHash, Bytes::new()>
    let mut all_codes_hashed = guest_program.codes_hashed.clone();

    let in_memory_store = InMemoryStore::new();

    // - Set up state trie nodes
    let all_nodes = &guest_program.nodes_hashed;
    let state_root_hash = guest_program.parent_block_header.state_root;

    let state_trie_nodes = InMemoryTrieDB::from_nodes(state_root_hash, all_nodes)?.inner;
    {
        // We now have the state trie built and we want 2 things:
        //   1. Add arbitrary Leaf nodes to the trie so that every reference in branch nodes point to an actual node.
        //   2. Get all code hashes that exist in the accounts that we have so that if we don't have the code we set it to empty bytes.
        // We do these things because sometimes the witness may be incomplete and in those cases we don't want failures for missing data.
        // This only applies when we use the InMemoryDatabase and not when we use the ExecutionWitness as database, that's because in the latter failures are dismissed and we fall back to default values.
        let mut state_nodes = state_trie_nodes.lock().unwrap();
        let referenced_node_hashes = get_referenced_hashes(&state_nodes)?;

        let dummy_leaf = Node::from(LeafNode::default()).encode_to_vec();
        // Insert arbitrary leaf nodes to state trie.
        for hash in referenced_node_hashes {
            state_nodes.entry(hash).or_insert(dummy_leaf.clone());
        }

        drop(state_nodes);

        let mut inner_store = in_memory_store.inner()?;

        inner_store.state_trie_nodes = state_trie_nodes;

        // - Set up storage trie nodes
        let addresses: Vec<Address> = witness
            .keys
            .iter()
            .filter(|k| k.len() == Address::len_bytes())
            .map(|k| Address::from_slice(k))
            .collect();

        for address in &addresses {
            let hashed_address = hash_address(address);

            // Account state may not be in the state trie
            let Some(account_state_rlp) = guest_program
                .state_trie
                .as_ref()
                .unwrap()
                .get(&hashed_address)?
            else {
                continue;
            };

            let account_state = AccountState::decode(&account_state_rlp)?;

            // If code hash of account isn't present insert empty code so that if not found the execution doesn't break.
            let code_hash = account_state.code_hash;
            all_codes_hashed.entry(code_hash).or_insert(vec![]);

            let storage_root = account_state.storage_root;
            let storage_trie = match InMemoryTrieDB::from_nodes(storage_root, all_nodes) {
                Ok(trie) => trie.inner,
                Err(_) => continue,
            };

            // Fill storage trie with dummy branch nodes that have the hash of the missing nodes
            // This is useful for eth_getProofs when we want to restructure the trie after removing a node whose sibling isn't known
            // We assume the sibling is a branch node because we already covered the cases in which it's a Leaf or Extension node by injecting nodes in the witness.
            // For more info read: https://github.com/kkrt-labs/zk-pig/blob/v0.8.0/docs/modified-mpt.md
            {
                let mut storage_nodes = storage_trie.lock().unwrap();
                let dummy_branch = Node::from(BranchNode::default()).encode_to_vec();

                let referenced_storage_node_hashes = get_referenced_hashes(&storage_nodes)?;

                for hash in referenced_storage_node_hashes {
                    storage_nodes.entry(hash).or_insert(dummy_branch.clone());
                }
            }

            inner_store
                .storage_trie_nodes
                .insert(H256::from_slice(&hashed_address), storage_trie);
        }
    }

    // Set up store with preloaded database and the right chain config.
    let store = Store {
        engine: Arc::new(in_memory_store),
        chain_config: Arc::new(RwLock::new(chain_config)),
        latest_block_header: Arc::new(RwLock::new(BlockHeader::default())),
    };

    // Add codes to DB
    for (code_hash, code) in all_codes_hashed {
        store.add_account_code(code_hash, code.into()).await?;
    }

    // Add block headers to DB
    for (_n, header) in guest_program.block_headers.clone() {
        store.add_block_header(header.hash(), header).await?;
    }

    let blockchain = Blockchain::default_with_store(store);

    info!("Storage preparation finished in {:.2?}", start.elapsed());

    info!("Executing block {} on {}", block.header.number, network);
    let start_time = Instant::now();
    blockchain.add_block(&block).await?;
    let duration = start_time.elapsed();
    info!("add_block execution time: {:.2?}", duration);

    Ok(duration)
}

async fn replay_transaction(tx_opts: TransactionOpts) -> eyre::Result<()> {
    let tx_hash = tx_opts.tx_hash;

    if tx_opts.opts.cached && tx_opts.block_number.is_none() {
        return Err(eyre::Error::msg(
            "In cached mode, --block-number must be specified for transaction replay",
        ));
    }

    let cache = if let Some(n) = tx_opts.block_number {
        get_blockdata(tx_opts.opts, Some(n)).await?.0
    } else {
        let (eth_client, _network) = setup_rpc(&tx_opts.opts).await?;
        // Get the block number of the transaction
        let tx = eth_client
            .get_transaction_by_hash(tx_hash)
            .await?
            .ok_or(eyre::Error::msg("error fetching transaction"))?;
        get_blockdata(tx_opts.opts, Some(tx.block_number.as_u64()))
            .await?
            .0
    };

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

    let (cache, network) = get_blockdata(opts.clone(), block).await?;

    let block =
        cache.blocks.first().cloned().ok_or_else(|| {
            eyre::Error::msg("no block found in the cache, this should never happen")
        })?;

    let backend = backend(&opts.common.zkvm)?;

    let (execution_result, proving_result) = if opts.no_zkvm {
        (replay_no_zkvm(cache.clone(), &opts).await, None)
    } else {
        // Always execute
        let execution_result = exec(backend, cache.clone()).await;

        let proving_result = if opts.common.action == Action::Prove {
            // Only prove if requested
            Some(prove(backend, cache.clone()).await)
        } else {
            None
        };

        (execution_result, proving_result)
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

    if opts.verbose {
        println!("{report}");
    } else {
        report.log();
    }

    try_send_report_to_slack(&report, opts.slack_webhook_url).await?;

    // Decide whether or not to keep the cache when fetching data from RPC.
    if !opts.cached {
        match opts.cache_level {
            // Cache is already saved
            CacheLevel::On => {}
            // Only save the cache if the block run or proving failed
            CacheLevel::Failed => {
                if report.execution_result.is_ok()
                    && report.proving_result.as_ref().is_none_or(|r| r.is_ok())
                {
                    cache.delete()?;
                }
            }
            // Don't keep the cache
            CacheLevel::Off => cache.delete()?,
        }
    }

    // CAUTION
    // This piece of code is used to create a benchmark file that is used by our
    // CI for updating benchmarks from https://docs.ethrex.xyz/benchmarks/.
    // Do no remove it under any circumstances, unless you are refactoring how
    // we do benchmarks in CI.
    if opts.bench {
        let benchmark_json = report.to_bench_file()?;
        let file =
            std::fs::File::create("bench_latest.json").expect("failed to create bench_latest.json");
        serde_json::to_writer(file, &benchmark_json)
            .map_err(|e| eyre::Error::msg(format!("failed to write to bench_latest.json: {e}")))?;
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
                Network::L2Chain(chain_id)
            } else {
                Network::LocalDevnet
            }
        }
    }
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

    let blockchain = Arc::new(Blockchain::new(
        store.clone(),
        ethrex_blockchain::BlockchainOptions::default(),
    ));

    let blocks = produce_l1_blocks(
        blockchain.clone(),
        &mut store,
        genesis.get_block().hash(),
        genesis.timestamp + 12,
        n_blocks,
    )
    .await?;

    let execution_witness = blockchain.generate_witness_for_blocks(&blocks).await?;
    let chain_config = execution_witness.chain_config;

    let cache = Cache::new(
        blocks,
        RpcExecutionWitness::from(execution_witness),
        chain_config,
        opts.cache_dir,
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

    let payload = create_payload(&build_payload_args, store, Bytes::new())?;

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
    opts: EthrexReplayOptions,
) -> eyre::Result<Report> {
    use ethrex_blockchain::{BlockchainOptions, BlockchainType};

    let network = Network::LocalDevnetL2;

    let genesis = network.get_genesis()?;

    let mut store = {
        let store_inner = Store::new("./", EngineType::InMemory)?;
        store_inner.add_initial_state(genesis.clone()).await?;
        store_inner
    };

    let rollup_store = {
        let rollup_store = StoreRollup::new(Path::new("./"), EngineTypeRollup::InMemory)
            .expect("Failed to create StoreRollup");
        rollup_store
            .init()
            .await
            .expect("Failed to init rollup store");
        rollup_store
    };

    let mut blockchain_options = BlockchainOptions::default();
    blockchain_options.r#type = BlockchainType::L2;
    let blockchain = Arc::new(Blockchain::new(store.clone(), blockchain_options));

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

    let cache = Cache::new(
        blocks,
        RpcExecutionWitness::from(execution_witness),
        genesis.config,
        opts.cache_dir.clone(),
    );

    let backend = backend(&opts.common.zkvm)?;

    let execution_result = exec(backend, cache.clone()).await;

    let proving_result = match opts.common.action {
        Action::Execute => None,
        Action::Prove => Some(prove(backend, cache.clone()).await),
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
    let mut last_privilege_nonce = None;

    for _ in 0..n_blocks {
        let block = produce_custom_l2_block(
            blockchain.clone(),
            store,
            rollup_store,
            current_parent_hash,
            current_timestamp,
            &mut last_privilege_nonce,
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
    last_privilege_nonce: &mut Option<u64>,
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

    let payload = create_payload(&build_payload_args, store, Bytes::new())?;

    let payload_build_result = build_payload(
        blockchain.clone(),
        payload,
        store,
        last_privilege_nonce,
        DEFAULT_BUILDER_GAS_CEIL,
    )
    .await?;

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

#[cfg(not(feature = "l2"))]
async fn fetch_latest_block_number(
    rpc_url: &Url,
    only_eth_proofs_blocks: bool,
) -> eyre::Result<u64> {
    let eth_client = EthClient::new(rpc_url.as_str())?;

    let mut latest_block_number = eth_client.get_block_number().await?.as_u64();

    while only_eth_proofs_blocks && latest_block_number % 100 != 0 {
        let blocks_left_for_next_eth_proofs_block = 100 - (latest_block_number % 100);

        let time_for_next_eth_proofs_block = Duration::from_secs(
            blocks_left_for_next_eth_proofs_block * 12, // assuming 12s block time
        );

        info!(
            "Latest block is {latest_block_number}, waiting for next eth proofs block ({}) in ~{}",
            latest_block_number + blocks_left_for_next_eth_proofs_block,
            format_duration(&time_for_next_eth_proofs_block)
        );

        tokio::time::sleep(time_for_next_eth_proofs_block).await;

        latest_block_number = eth_client.get_block_number().await?.as_u64();
    }

    Ok(latest_block_number)
}

fn format_duration(duration: &Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = duration.subsec_millis();

    if hours > 0 {
        return format!("{hours:02}h {minutes:02}m {seconds:02}s {milliseconds:03}ms");
    }

    if minutes == 0 {
        return format!("{seconds:02}s {milliseconds:03}ms");
    }

    format!("{minutes:02}m {seconds:02}s")
}
