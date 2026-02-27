use crate::{
    cli::{DB_ETHREX_DEV_L1, DB_ETHREX_DEV_L2, remove_db},
    initializers::{init_l1, init_store, init_tracing},
    l2::{
        self,
        deployer::{DeployerOptions, deploy_l1_contracts},
        options::{Options, ProverClientOptions, parse_signer},
    },
    utils::{self, default_datadir, init_datadir, parse_private_key},
};
use bytes::Bytes;
use clap::{FromArgMatches, Parser, Subcommand};
use ethrex_blockchain::{
    Blockchain, BlockchainOptions, BlockchainType, L2Config, fork_choice::apply_fork_choice,
};
use ethrex_common::{
    Address, U256,
    types::{BYTES_PER_BLOB, Block, blobs_bundle, bytes_from_blob, fee_config::FeeConfig},
};
use ethrex_common::{types::BlobsBundle, utils::keccak};
use ethrex_config::networks::Network;
use ethrex_l2::utils::state_reconstruct::get_batch;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_sdk::call_contract;
use ethrex_rlp::decode::RLPDecode as _;
use ethrex_rpc::{
    EthClient, clients::beacon::BeaconClient, types::block_identifier::BlockIdentifier,
};
use ethrex_storage::{EngineType, Store};
use ethrex_storage_rollup::StoreRollup;
use eyre::OptionExt;
use itertools::Itertools;
use reqwest::Url;
use secp256k1::{PublicKey, SecretKey};
use std::{
    fs::{create_dir_all, read_dir},
    path::{Path, PathBuf},
    time::Duration,
};
use tracing::{debug, info};

// Compile-time check to ensure that at least one of the database features is enabled.
#[cfg(not(feature = "rocksdb"))]
const _: () = {
    compile_error!("Database feature must be enabled (Available: `rocksdb`).");
};

const PAUSE_CONTRACT_SELECTOR: &str = "pause()";
const UNPAUSE_CONTRACT_SELECTOR: &str = "unpause()";
const REVERT_BATCH_SELECTOR: &str = "revertBatch(uint256)";
#[derive(Parser)]
#[clap(args_conflicts_with_subcommands = true)]
pub struct L2Command {
    #[clap(subcommand)]
    pub command: Option<Command>,
    #[clap(flatten)]
    pub opts: Option<Options>,
}

impl L2Command {
    pub async fn run(self) -> eyre::Result<()> {
        if let Some(cmd) = self.command {
            return cmd.run().await;
        }
        let mut app = clap::Command::new("init");
        app = <Options as clap::Args>::augment_args(app);

        let args = std::env::args().skip(2).collect::<Vec<_>>();
        let args_with_program = std::iter::once("init".to_string())
            .chain(args.into_iter())
            .collect::<Vec<_>>();

        let matches = app.try_get_matches_from(args_with_program)?;
        let init_options = Options::from_arg_matches(&matches)?;
        let (log_filter_handler, _guard) = l2::init_tracing(&init_options);
        let mut l2_options = init_options;

        if l2_options.node_opts.dev {
            println!("Removing L1 and L2 databases...");
            remove_db(DB_ETHREX_DEV_L1.as_ref(), true);
            remove_db(DB_ETHREX_DEV_L2.as_ref(), true);
            println!("Initializing L1");
            init_l1(
                crate::cli::Options::default_l1(),
                log_filter_handler.clone(),
            )
            .await?;
            println!("Deploying contracts...");
            let contract_addresses =
                l2::deployer::deploy_l1_contracts(l2::deployer::DeployerOptions::default()).await?;

            l2_options.node_opts = crate::cli::Options::default_l2();
            l2_options.populate_with_defaults();
            l2_options
                .sequencer_opts
                .committer_opts
                .on_chain_proposer_address = Some(contract_addresses.on_chain_proposer_address);
            l2_options.sequencer_opts.committer_opts.timelock_address =
                contract_addresses.timelock_address;
            l2_options.sequencer_opts.watcher_opts.bridge_address =
                Some(contract_addresses.bridge_address);
            println!("Initializing L2");
        }
        #[cfg(feature = "native-rollups")]
        if l2_options.sequencer_opts.native_rollup_opts.enabled {
            l2::initializers::init_native_rollup_l2(l2_options, log_filter_handler).await?;
            return Ok(());
        }
        l2::init_l2(l2_options, log_filter_handler).await?;
        Ok(())
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
#[clap(group = clap::ArgGroup::new("owner_signing").required(false))]
#[clap(group = clap::ArgGroup::new("sequencer_signing").required(false))]
pub enum Command {
    #[command(about = "Initialize an ethrex prover", visible_alias = "p")]
    Prover {
        #[command(flatten)]
        prover_client_options: ProverClientOptions,
    },
    #[command(name = "removedb", about = "Remove the database", visible_aliases = ["rm", "clean"])]
    RemoveDB {
        #[arg(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value = default_datadir().into_os_string(), required = false)]
        datadir: PathBuf,
        #[arg(long = "force", required = false, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    #[command(about = "Launch a server that listens for Blobs submissions and saves them offline.")]
    BlobsSaver {
        #[arg(
            short = 'c',
            long = "contract",
            help = "The contract address to listen to."
        )]
        contract_address: Address,
        #[arg(short = 'd', long, help = "The directory to save the blobs.")]
        datadir: PathBuf,
        #[arg(short = 'e', long)]
        l1_eth_rpc: Url,
        #[arg(short = 'b', long)]
        l1_beacon_rpc: Url,
    },
    #[command(about = "Reconstructs the L2 state from L1 blobs.")]
    Reconstruct {
        #[arg(short = 'g', long, help = "The genesis file for the L2 network.")]
        genesis: PathBuf,
        #[arg(short = 'b', long, help = "The directory to read the blobs from.")]
        blobs_dir: PathBuf,
        #[arg(short = 's', long, help = "The path to the store.")]
        store_path: PathBuf,
        #[arg(
            short = 'o',
            long,
            help = "Whether Osaka fork is activated or not. If None, it assumes it is active."
        )]
        osaka_activated: Option<bool>,
    },
    #[command(about = "Reverts unverified batches.")]
    RevertBatch {
        #[arg(help = "ID of the batch to revert. This and all following batches will be removed.")]
        batch: u64,
        #[arg(
            long = "datadir",
            value_name = "DATABASE_DIRECTORY",
            default_value = default_datadir().into_os_string(),
            help = "Receives the name of the directory where the Database is located.",
            env = "ETHREX_DATADIR"
        )]
        datadir: PathBuf,
        #[arg(
            long = "pause",
            default_value_t = false,
            help = "Pause contracts before trying to revert the batch",
            requires = "owner_signing"
        )]
        pause_contracts: bool,
        #[arg(
            long,
            default_value = "http://localhost:8545",
            env = "RPC_URL",
            help = "URL of the L1 RPC"
        )]
        rpc_url: Url,
        #[arg(help = "The address of the OnChainProposer contract")]
        contract_address: Address,
        #[arg(
            long,
            value_parser = parse_private_key,
            env = "OWNER_PRIVATE_KEY",
            help = "The private key of the owner",
            help_heading  = "Contract owner account options",
            group = "owner_signing",
        )]
        owner_private_key: Option<SecretKey>,
        #[arg(
            long = "owner-remote-signer-url",
            value_name = "URL",
            env = "OWNER_REMOTE_SIGNER_URL",
            help = "URL of a Web3Signer-compatible server to remote sign instead of a local private key.",
            help_heading = "Contract owner account options",
            conflicts_with = "owner_private_key",
            requires = "owner_remote_signer_public_key"
        )]
        owner_remote_signer_url: Option<Url>,
        #[arg(
            long = "owner-remote-signer-public-key",
            value_name = "OWNER_PUBLIC_KEY",
            value_parser = utils::parse_public_key,
            env = "ETHREX_REMOTE_SIGNER_PUBLIC_KEY",
            help = "Public key to request the remote signature from.",
            group = "owner_signing",
            requires = "owner_remote_signer_url",
            help_heading  = "Contract owner account options"
        )]
        owner_remote_signer_public_key: Option<PublicKey>,
        #[arg(
            long,
            value_parser = parse_private_key,
            env = "SEQUENCER_PRIVATE_KEY",
            help = "The private key of the sequencer",
            help_heading  = "Sequencer account options",
            group = "sequencer_signing",
        )]
        sequencer_private_key: Option<SecretKey>,
        #[arg(
            long = "sequencer-remote-signer-url",
            value_name = "URL",
            env = "SEQUENCER_REMOTE_SIGNER_URL",
            help = "URL of a Web3Signer-compatible server to remote sign instead of a local private key.",
            help_heading = "Sequencer account options",
            conflicts_with = "sequencer_private_key",
            requires = "sequencer_remote_signer_public_key"
        )]
        sequencer_remote_signer_url: Option<Url>,
        #[arg(
            long = "sequencer-remote-signer-public-key",
            value_name = "SEQUENCER_PUBLIC_KEY",
            value_parser = utils::parse_public_key,
            env = "SEQUENCER_REMOTE_SIGNER_PUBLIC_KEY",
            help = "Public key to request the remote signature from.",
            group = "sequencer_signing",
            requires = "sequencer_remote_signer_url",
            help_heading  = "Sequencer account options"
        )]
        sequencer_remote_signer_public_key: Option<PublicKey>,
        #[arg(
            default_value_t = false,
            help = "If enabled the command will also delete the blocks from the Blockchain database",
            long = "delete-blocks",
            requires = "network"
        )]
        delete_blocks: bool,
        #[arg(
            long = "network",
            value_name = "GENESIS_FILE_PATH",
            help = "Receives a `Genesis` struct in json format. Only required if using --delete-blocks",
            env = "ETHREX_NETWORK",
            value_parser = clap::value_parser!(Network),
        )]
        network: Option<Network>,
    },
    #[command(about = "Pause L1 contracts")]
    Pause {
        #[command(flatten)]
        contract_call_options: ContractCallOptions,
    },
    #[command(about = "Unpause L1 contracts")]
    Unpause {
        #[command(flatten)]
        contract_call_options: ContractCallOptions,
    },
    #[command(about = "Deploy in L1 all contracts needed by an L2.")]
    Deploy {
        #[command(flatten)]
        options: DeployerOptions,
    },
}

impl Command {
    pub async fn run(self) -> eyre::Result<()> {
        match &self {
            Command::Prover {
                prover_client_options,
            } => init_tracing(&crate::cli::Options {
                log_level: prover_client_options.log_level,
                ..Default::default()
            }),
            _ => init_tracing(&crate::cli::Options::default()),
        };

        match self {
            Command::Prover {
                prover_client_options,
            } => ethrex_prover_lib::init_client(prover_client_options.into()).await,
            Self::RemoveDB { datadir, force } => {
                remove_db(&datadir, force);
            }
            Command::BlobsSaver {
                l1_eth_rpc,
                l1_beacon_rpc,
                contract_address,
                datadir,
            } => {
                create_dir_all(datadir.clone())?;

                let eth_client = EthClient::new(l1_eth_rpc)?;
                let beacon_client = BeaconClient::new(l1_beacon_rpc);

                // Keep delay for finality
                let mut current_block = U256::zero();
                while current_block < U256::from(64) {
                    current_block = eth_client.get_block_number().await?;
                    tokio::time::sleep(Duration::from_secs(12)).await;
                }
                current_block = current_block
                    .checked_sub(U256::from(64))
                    .ok_or_eyre("Cannot get finalized block")?;

                let event_signature = keccak("BatchCommitted(bytes32)");

                loop {
                    // Wait for a block
                    tokio::time::sleep(Duration::from_secs(12)).await;

                    let logs = eth_client
                        .get_logs(
                            current_block,
                            current_block,
                            contract_address,
                            vec![event_signature],
                        )
                        .await?;

                    if !logs.is_empty() {
                        // Get parent beacon block root hash from block
                        let block = eth_client
                            .get_block_by_number(
                                BlockIdentifier::Number(current_block.as_u64()),
                                false,
                            )
                            .await?;
                        let parent_beacon_hash = block
                            .header
                            .parent_beacon_block_root
                            .ok_or_eyre("Unknown parent beacon root")?;

                        // Get block slot from parent beacon block
                        let parent_beacon_block =
                            beacon_client.get_block_by_hash(parent_beacon_hash).await?;
                        let target_slot = parent_beacon_block.message.slot + 1;

                        // Get versioned hashes from transactions
                        let mut l2_blob_hashes = vec![];
                        for log in logs {
                            let tx = eth_client
                                .get_transaction_by_hash(log.transaction_hash)
                                .await?
                                .ok_or_eyre(format!(
                                    "Transaction {:#x} not found",
                                    log.transaction_hash
                                ))?;
                            l2_blob_hashes.extend(tx.tx.blob_versioned_hashes());
                        }

                        // Get blobs from block's slot and only keep L2 commitment's blobs
                        for blob in beacon_client
                            .get_blobs_by_slot(target_slot)
                            .await?
                            .into_iter()
                            .filter(|blob| l2_blob_hashes.contains(&blob.versioned_hash()))
                        {
                            let blob_path =
                                datadir.join(format!("{target_slot}-{}.blob", blob.index));
                            std::fs::write(blob_path, blob.blob)?;
                        }

                        println!("Saved blobs for slot {target_slot}");
                    }

                    current_block += U256::one();
                }
            }
            Command::Reconstruct {
                genesis,
                blobs_dir,
                store_path,
                osaka_activated,
            } => {
                #[cfg(feature = "rocksdb")]
                let store_type = EngineType::RocksDB;

                #[cfg(feature = "l2-sql")]
                let rollup_store_type = ethrex_storage_rollup::EngineTypeRollup::SQL;
                #[cfg(not(feature = "l2-sql"))]
                let rollup_store_type = ethrex_storage_rollup::EngineTypeRollup::InMemory;

                // Init stores
                let store = Store::new_from_genesis(
                    &store_path,
                    store_type,
                    genesis.to_str().expect("Invalid genesis path"),
                )
                .await?;

                let chain_id = store.get_chain_config().chain_id;

                let rollup_store =
                    StoreRollup::new(&store_path.join("rollup_store"), rollup_store_type)?;
                rollup_store
                    .init()
                    .await
                    .map_err(|e| format!("Failed to init rollup store: {e}"))
                    .unwrap();

                // Iterate over each blob
                let files: Vec<std::fs::DirEntry> = read_dir(blobs_dir)?.try_collect()?;
                for (file_number, file) in files
                    .into_iter()
                    .sorted_by_key(|f| f.file_name())
                    .enumerate()
                {
                    let batch_number = file_number as u64 + 1;
                    let blob = std::fs::read(file.path())?;

                    if blob.len() != BYTES_PER_BLOB {
                        panic!("Invalid blob size");
                    }

                    let blob = bytes_from_blob(blob.into());

                    // Decode blocks
                    let blocks_count = u64::from_be_bytes(
                        blob[0..8].try_into().expect("Failed to get blob length"),
                    );

                    let mut buf = &blob[8..];
                    let mut blocks = Vec::new();
                    for _ in 0..blocks_count {
                        let (item, rest) = Block::decode_unfinished(buf)?;
                        blocks.push(item);
                        buf = rest;
                    }

                    // Decode fee configs
                    let mut fee_configs = Vec::new();

                    for _ in 0..blocks_count {
                        let (consumed, fee_config) = FeeConfig::decode(buf)?;
                        fee_configs.push(fee_config);
                        buf = &buf[consumed..];
                    }

                    // Create blockchain to execute blocks
                    let blockchain_type =
                        ethrex_blockchain::BlockchainType::L2(L2Config::default());
                    let opts = BlockchainOptions {
                        r#type: blockchain_type,
                        ..Default::default()
                    };
                    let blockchain = Blockchain::new(store.clone(), opts);

                    for (i, block) in blocks.iter().enumerate() {
                        // Update blockchain with the block's fee config
                        let fee_config = fee_configs
                            .get(i)
                            .cloned()
                            .ok_or_eyre("Fee config not found for block")?;

                        let BlockchainType::L2(l2_config) = &blockchain.options.r#type else {
                            panic!("Invalid blockchain type. Expected L2.");
                        };

                        {
                            let Ok(mut fee_config_guard) = l2_config.fee_config.write() else {
                                panic!("Fee config lock was poisoned.");
                            };

                            *fee_config_guard = fee_config;
                        }

                        // Execute block
                        blockchain.add_block_pipeline(block.clone())?;

                        // Add fee config to rollup store
                        rollup_store
                            .store_fee_config_by_block(block.header.number, fee_config)
                            .await?;

                        info!(
                            "Added block {} with hash {:#x}",
                            block.header.number,
                            block.hash(),
                        );
                    }
                    // Apply fork choice
                    let latest_hash_on_batch = blocks.last().ok_or_eyre("Batch is empty")?.hash();
                    apply_fork_choice(
                        &store,
                        latest_hash_on_batch,
                        latest_hash_on_batch,
                        latest_hash_on_batch,
                    )
                    .await?;

                    // Prepare batch sealing
                    let blob = blobs_bundle::blob_from_bytes(Bytes::copy_from_slice(&blob))
                        .expect("Failed to create blob from bytes; blob was just read from file");

                    let wrapper_version = if let Some(activated) = osaka_activated
                        && !activated
                    {
                        None
                    } else {
                        Some(1)
                    };

                    let blobs_bundle =
                        BlobsBundle::create_from_blobs(&vec![blob], wrapper_version)?;

                    let batch = get_batch(
                        &store,
                        &blocks,
                        U256::from(batch_number),
                        None,
                        blobs_bundle,
                        chain_id,
                    )
                    .await?;

                    // Seal batch
                    rollup_store.seal_batch(batch).await?;

                    // Create checkpoint
                    let checkpoint_path =
                        store_path.join(format!("checkpoint_batch_{batch_number}"));
                    store.create_checkpoint(&checkpoint_path)?;

                    info!("Sealed batch {batch_number}.");
                }
            }
            Command::RevertBatch {
                batch,
                datadir,
                network,
                contract_address,
                owner_private_key,
                owner_remote_signer_public_key,
                owner_remote_signer_url,
                sequencer_private_key,
                sequencer_remote_signer_public_key,
                sequencer_remote_signer_url,
                rpc_url,
                delete_blocks,
                pause_contracts,
            } => {
                init_datadir(&datadir);
                let rollup_store_dir = datadir.join("rollup_store");
                let owner_contract_options = ContractCallOptions {
                    contract_address,
                    private_key: owner_private_key,
                    remote_signer_public_key: owner_remote_signer_public_key,
                    remote_signer_url: owner_remote_signer_url,
                    rpc_url: rpc_url.clone(),
                };
                let sequencer_contract_options = if sequencer_private_key.is_some()
                    || sequencer_remote_signer_public_key.is_some()
                {
                    Some(ContractCallOptions {
                        contract_address,
                        private_key: sequencer_private_key,
                        remote_signer_public_key: sequencer_remote_signer_public_key,
                        remote_signer_url: sequencer_remote_signer_url,
                        rpc_url,
                    })
                } else {
                    None
                };
                if pause_contracts {
                    info!("Pausing OnChainProposer contract");
                    owner_contract_options
                        .call_contract(PAUSE_CONTRACT_SELECTOR, vec![])
                        .await?;
                    info!("Paused OnChainProposer contract");
                }
                if let Some(contract_opts) = sequencer_contract_options.as_ref() {
                    info!("Doing revert on OnChainProposer...");
                    contract_opts
                        .call_contract(REVERT_BATCH_SELECTOR, vec![Value::Uint(batch.into())])
                        .await?;
                    info!("Reverted batch {batch} on OnChainProposer")
                } else {
                    info!("Private key not given, not updating contract.");
                }

                let last_kept_block =
                    delete_batch_from_rollup_store(batch, &rollup_store_dir).await?;

                if delete_blocks {
                    delete_blocks_from_batch(&datadir, network, last_kept_block).await?;
                }

                if pause_contracts {
                    info!("Unpausing OnChainProposer contract");
                    owner_contract_options
                        .call_contract(UNPAUSE_CONTRACT_SELECTOR, vec![])
                        .await?;
                    info!("Unpaused OnChainProposer contract");
                }
            }
            Command::Pause {
                contract_call_options: opts,
            } => {
                info!("Pausing contract {}", opts.contract_address);
                opts.call_contract(PAUSE_CONTRACT_SELECTOR, vec![])
                    .await
                    .inspect(|_| info!("Succesfully paused contract"))?;
            }
            Command::Unpause {
                contract_call_options: opts,
            } => {
                info!("Unpausing contract {}", opts.contract_address);
                opts.call_contract(UNPAUSE_CONTRACT_SELECTOR, vec![])
                    .await
                    .inspect(|_| info!("Succesfully unpaused contract"))?;
            }
            Command::Deploy { options } => {
                #[cfg(feature = "native-rollups")]
                if options.native_rollups {
                    l2::deployer::deploy_native_rollup_contracts(options).await?;
                    return Ok(());
                }
                deploy_l1_contracts(options).await?;
            }
        }
        Ok(())
    }
}

#[derive(Parser)]
pub struct ContractCallOptions {
    #[arg(help = "The address of the target contract")]
    contract_address: Address,
    #[arg(long, value_parser = parse_private_key, env = "PRIVATE_KEY", help = "The private key of the owner. Assumed to have sequencing permission.")]
    private_key: Option<SecretKey>,
    #[arg(
        long,
        default_value = "http://localhost:8545",
        env = "RPC_URL",
        help = "URL of the L1 RPC"
    )]
    rpc_url: Url,
    #[arg(
        long = "remote-signer-url",
        value_name = "URL",
        env = "ETHREX_REMOTE_SIGNER_URL",
        help = "URL of a Web3Signer-compatible server to remote sign instead of a local private key.",
        requires = "remote_signer_public_key",
        conflicts_with = "private_key"
    )]
    remote_signer_url: Option<Url>,
    #[arg(
            long = "remote-signer-public-key",
            value_name = "PUBLIC_KEY",
            value_parser = utils::parse_public_key,
            env = "ETHREX_REMOTE_SIGNER_PUBLIC_KEY",
            help = "Public key to request the remote signature from.",
            requires = "remote_signer_url",
            conflicts_with = "private_key"
        )]
    remote_signer_public_key: Option<PublicKey>,
}

impl ContractCallOptions {
    async fn call_contract(&self, selector: &str, params: Vec<Value>) -> eyre::Result<()> {
        let client = EthClient::new(self.rpc_url.clone())?;
        let signer = parse_signer(
            self.private_key,
            self.remote_signer_url.clone(),
            self.remote_signer_public_key,
        )?;

        call_contract(&client, &signer, self.contract_address, selector, params).await?;
        Ok(())
    }
}

async fn delete_batch_from_rollup_store(batch: u64, rollup_store_dir: &Path) -> eyre::Result<u64> {
    info!("Deleting batch from rollup store...");
    let rollup_store = l2::initializers::init_rollup_store(rollup_store_dir).await;
    let last_kept_block = rollup_store
        .get_block_numbers_by_batch(batch)
        .await?
        .and_then(|kept_blocks| kept_blocks.iter().max().cloned())
        .unwrap_or(0);
    rollup_store.revert_to_batch(batch).await?;
    info!("Succesfully deleted batch from rollup store");
    Ok(last_kept_block)
}

async fn delete_blocks_from_batch(
    datadir: &Path,
    network: Option<Network>,
    last_kept_block: u64,
) -> eyre::Result<()> {
    info!("Deleting blocks from blockchain store...");
    let Some(network) = network else {
        return Err(eyre::eyre!("Network not provided"));
    };
    let genesis = network.get_genesis()?;

    let mut block_to_delete = last_kept_block + 1;
    let store = init_store(datadir, genesis).await?;

    while store
        .get_canonical_block_hash(block_to_delete)
        .await?
        .is_some()
    {
        debug!("Deleting block {block_to_delete}");
        store.remove_block(block_to_delete).await?;
        block_to_delete += 1;
    }
    let last_kept_header = store
        .get_block_header(last_kept_block)?
        .ok_or_else(|| eyre::eyre!("Block number {} not found", last_kept_block))?;
    store
        .forkchoice_update(vec![], last_kept_block, last_kept_header.hash(), None, None)
        .await?;
    Ok(())
}
