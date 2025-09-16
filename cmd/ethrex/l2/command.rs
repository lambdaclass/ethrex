use crate::{
    cli::remove_db,
    initializers::{init_l1, init_store, init_tracing},
    l2::{
        self,
        deployer::{DeployerOptions, deploy_l1_contracts},
        options::{Options, ProverClientOptions, parse_signer},
    },
    utils::{self, default_datadir, init_datadir, parse_private_key},
};
use clap::{FromArgMatches, Parser, Subcommand};
use ethrex_common::{
    Address, H256, U256,
    types::{BYTES_PER_BLOB, BlobsBundle, BlockHeader, batch::Batch, bytes_from_blob},
};
use ethrex_config::networks::Network;
use ethrex_l2_common::{calldata::Value, l1_messages::get_l1_message_hash, state_diff::StateDiff};
use ethrex_l2_sdk::call_contract;
use ethrex_rpc::{
    EthClient, clients::beacon::BeaconClient, types::block_identifier::BlockIdentifier,
};
use ethrex_storage::{EngineType, Store, UpdateBatch};
use ethrex_storage_rollup::StoreRollup;
use eyre::OptionExt;
use itertools::Itertools;
use keccak_hash::keccak;
use reqwest::Url;
use secp256k1::{PublicKey, SecretKey};
use std::{
    fs::{create_dir_all, read_dir},
    path::PathBuf,
    time::Duration,
};
use tracing::{debug, info};

pub const DB_ETHREX_DEV_L1: &str = "dev_ethrex_l1";
pub const DB_ETHREX_DEV_L2: &str = "dev_ethrex_l2";

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
        let log_filter_handler = l2::init_tracing(&init_options);
        let mut l2_options = init_options;

        if l2_options.node_opts.dev {
            println!("Removing L1 and L2 databases...");
            remove_db(DB_ETHREX_DEV_L1, true);
            remove_db(DB_ETHREX_DEV_L2, true);
            println!("Initializing L1");
            init_l1(
                crate::cli::Options::default_l1(),
                log_filter_handler.clone(),
            )
            .await?;
            println!("Deploying contracts...");
            let contract_addresses =
                l2::deployer::deploy_l1_contracts(l2::deployer::DeployerOptions::default()).await?;

            l2_options = l2::options::Options {
                node_opts: crate::cli::Options::default_l2(),
                ..Default::default()
            };
            l2_options
                .sequencer_opts
                .committer_opts
                .on_chain_proposer_address = Some(contract_addresses.on_chain_proposer_address);
            l2_options.sequencer_opts.watcher_opts.bridge_address =
                Some(contract_addresses.bridge_address);
            println!("Initializing L2");
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
        #[arg(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value_t = default_datadir(), required = false)]
        datadir: String,
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
        data_dir: PathBuf,
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
        #[arg(short = 'c', long, help = "Address of the L2 proposer coinbase")]
        coinbase: Address,
    },
    #[command(about = "Reverts unverified batches.")]
    RevertBatch {
        #[arg(help = "ID of the batch to revert to")]
        batch: u64,
        #[arg(
            long = "datadir",
            value_name = "DATABASE_DIRECTORY",
            default_value_t = default_datadir(),
            help = "Receives the name of the directory where the Database is located.",
            env = "ETHREX_DATADIR"
        )]
        datadir: String,
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
                data_dir,
            } => {
                create_dir_all(data_dir.clone())?;

                let eth_client = EthClient::new(l1_eth_rpc.as_str())?;
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
                            l2_blob_hashes.extend(tx.blob_versioned_hashes.ok_or_eyre(format!(
                                "Blobs not found in transaction {:#x}",
                                log.transaction_hash
                            ))?);
                        }

                        // Get blobs from block's slot and only keep L2 commitment's blobs
                        for blob in beacon_client
                            .get_blobs_by_slot(target_slot)
                            .await?
                            .into_iter()
                            .filter(|blob| l2_blob_hashes.contains(&blob.versioned_hash()))
                        {
                            let blob_path =
                                data_dir.join(format!("{target_slot}-{}.blob", blob.index));
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
                coinbase,
            } => {
                cfg_if::cfg_if! {
                    if #[cfg(feature = "libmdbx")] {
                        let store_type = EngineType::Libmdbx;
                    } else {
                        eyre::bail!("Expected libmdbx store engine");
                    }
                };
                cfg_if::cfg_if! {
                    if #[cfg(feature = "rollup_storage_sql")] {
                        let rollup_store_type = ethrex_storage_rollup::EngineTypeRollup::SQL;
                    } else {
                        eyre::bail!("Expected sql rollup store engine");
                    }
                };

                // Init stores
                let store = Store::new_from_genesis(
                    store_path.to_str().expect("Invalid store path"),
                    store_type,
                    genesis.to_str().expect("Invalid genesis path"),
                )
                .await?;

                let rollup_store = StoreRollup::new(
                    store_path
                        .join("./rollup_store")
                        .to_str()
                        .expect("Invalid store path"),
                    rollup_store_type,
                )?;
                rollup_store
                    .init()
                    .await
                    .map_err(|e| format!("Failed to init rollup store: {e}"))
                    .unwrap();

                // Get genesis
                let genesis_header = store.get_block_header(0)?.expect("Genesis block not found");
                let genesis_block_hash = genesis_header.hash();

                let mut new_trie = store
                    .state_trie(genesis_block_hash)?
                    .expect("Genesis block not found");

                let mut last_block_number = 0;
                let mut new_canonical_blocks = vec![];

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

                    // Decode state diff from blob
                    let blob = bytes_from_blob(blob.into());
                    let state_diff = StateDiff::decode(&blob)?;

                    // Apply all account updates to trie
                    let account_updates = state_diff.to_account_updates(&new_trie)?;
                    let account_updates_list = store
                        .apply_account_updates_from_trie_batch(new_trie, account_updates.values())
                        .await
                        .map_err(|e| format!("Error applying account updates: {e}"))
                        .unwrap();

                    let (new_state_root, state_updates, accounts_updates) = (
                        account_updates_list.state_trie_hash,
                        account_updates_list.state_updates,
                        account_updates_list.storage_updates,
                    );

                    let pseudo_update_batch = UpdateBatch {
                        account_updates: state_updates,
                        storage_updates: accounts_updates,
                        blocks: vec![],
                        receipts: vec![],
                        code_updates: vec![],
                    };

                    store
                        .store_block_updates(pseudo_update_batch)
                        .await
                        .map_err(|e| format!("Error storing trie updates: {e}"))
                        .unwrap();

                    new_trie = store
                        .open_state_trie(new_state_root)
                        .map_err(|e| format!("Error opening new state trie: {e}"))
                        .unwrap();

                    // Get withdrawal hashes
                    let message_hashes = state_diff
                        .l1_messages
                        .iter()
                        .map(get_l1_message_hash)
                        .collect();

                    // Get the first block of the batch
                    let first_block_number = last_block_number + 1;

                    // Build the header of the last block.
                    // Note that its state_root is the root of new_trie.
                    let new_block = BlockHeader {
                        coinbase,
                        state_root: new_trie
                            .hash()
                            .map_err(|e| format!("Error committing state: {e}"))
                            .unwrap(),
                        ..state_diff.last_header
                    };

                    // Store last block.
                    let new_block_hash = new_block.hash();
                    store
                        .add_block_header(new_block_hash, new_block.clone())
                        .await?;
                    store
                        .add_block_number(new_block_hash, state_diff.last_header.number)
                        .await?;
                    new_canonical_blocks.push((state_diff.last_header.number, new_block_hash));
                    println!(
                        "Stored last block of blob. Block {}. State root {}",
                        new_block.number, new_block.state_root
                    );

                    last_block_number = new_block.number;

                    let batch = Batch {
                        number: batch_number,
                        first_block: first_block_number,
                        last_block: new_block.number,
                        state_root: new_block.state_root,
                        privileged_transactions_hash: H256::zero(),
                        message_hashes,
                        blobs_bundle: BlobsBundle::empty(),
                        commit_tx: None,
                        verify_tx: None,
                    };

                    // Store batch info in L2 storage
                    rollup_store
                        .seal_batch(batch)
                        .await
                        .map_err(|e| format!("Error storing batch: {e}"))
                        .unwrap();
                }
                let Some((last_number, last_hash)) = new_canonical_blocks.pop() else {
                    return Err(eyre::eyre!("No blocks found in blobs directory"));
                };
                store
                    .forkchoice_update(
                        Some(new_canonical_blocks),
                        last_number,
                        last_hash,
                        None,
                        None,
                    )
                    .await?;
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
                let data_dir = init_datadir(&datadir);
                let rollup_store_dir = data_dir.clone() + "/rollup_store";
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
                    info!("Reverted to batch {batch} on OnChainProposer")
                } else {
                    info!("Private key not given, not updating contract.");
                }

                let last_kept_block =
                    delete_batch_from_rollup_store(batch, &rollup_store_dir).await?;

                if delete_blocks {
                    delete_blocks_from_batch(&data_dir, network, last_kept_block).await?;
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
        let client = EthClient::new(self.rpc_url.as_str())?;
        let signer = parse_signer(
            self.private_key,
            self.remote_signer_url.clone(),
            self.remote_signer_public_key,
        )?;

        call_contract(&client, &signer, self.contract_address, selector, params).await?;
        Ok(())
    }
}

async fn delete_batch_from_rollup_store(batch: u64, rollup_store_dir: &str) -> eyre::Result<u64> {
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
    data_dir: &str,
    network: Option<Network>,
    last_kept_block: u64,
) -> eyre::Result<()> {
    info!("Deleting blocks from blockchain store...");
    let Some(network) = network else {
        return Err(eyre::eyre!("Network not provided"));
    };
    let genesis = network.get_genesis()?;

    let mut block_to_delete = last_kept_block + 1;
    let store = init_store(data_dir, genesis).await;

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
        .forkchoice_update(None, last_kept_block, last_kept_header.hash(), None, None)
        .await?;
    Ok(())
}
