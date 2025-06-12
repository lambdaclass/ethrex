use crate::{
    cli::{self as ethrex_cli, Options as NodeOptions},
    initializers::{
        get_local_node_record, get_local_p2p_node, get_network, get_signer, init_blockchain,
        init_metrics, init_network, init_rollup_store, init_rpc_api, init_store,
    },
    l2::options::Options,
    utils::{set_datadir, store_node_config_file, NodeConfigFile},
    DEFAULT_L2_DATADIR,
};
use clap::Subcommand;
use ethrex_common::{
    types::{batch::Batch, bytes_from_blob, BlobsBundle, BlockHeader, BYTES_PER_BLOB},
    Address, U256,
};
use ethrex_l2::SequencerConfig;
use ethrex_l2_common::state_diff::StateDiff;
use ethrex_p2p::network::peer_table;
use ethrex_rpc::{
    clients::{beacon::BeaconClient, eth::BlockByNumber},
    EthClient,
};
use ethrex_storage::{EngineType, Store, UpdateBatch};
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use ethrex_vm::EvmEngine;
use eyre::OptionExt;
use itertools::Itertools;
use keccak_hash::keccak;
use reqwest::Url;
use std::{
    fs::{create_dir_all, read_dir},
    future::IntoFuture,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::info;

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum Command {
    #[command(about = "Initialize an ethrex L2 node", visible_alias = "i")]
    Init {
        #[command(flatten)]
        opts: Options,
    },
    #[command(name = "removedb", about = "Remove the database", visible_aliases = ["rm", "clean"])]
    RemoveDB {
        #[arg(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value = DEFAULT_L2_DATADIR, required = false)]
        datadir: String,
        #[arg(long = "force", required = false, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    #[command(
        about = "Launch a server that listens for Blobs submissions and saves them offline."
    )]
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
}

impl Command {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            Command::Init { opts } => {
                if opts.node_opts.evm == EvmEngine::REVM {
                    panic!("L2 Doesn't support REVM, use LEVM instead.");
                }

                let data_dir = set_datadir(&opts.node_opts.datadir);
                let rollup_store_dir = data_dir.clone() + "/rollup_store";

                let network = get_network(&opts.node_opts);

                let genesis = network.get_genesis();
                let store = init_store(&data_dir, genesis).await;
                let rollup_store = init_rollup_store(&rollup_store_dir).await;

                let blockchain = init_blockchain(opts.node_opts.evm, store.clone());

                let signer = get_signer(&data_dir);

                let local_p2p_node = get_local_p2p_node(&opts.node_opts, &signer);

                let local_node_record = Arc::new(Mutex::new(get_local_node_record(
                    &data_dir,
                    &local_p2p_node,
                    &signer,
                )));

                let peer_table = peer_table(local_p2p_node.node_id());

                // TODO: Check every module starts properly.
                let tracker = TaskTracker::new();

                let cancel_token = tokio_util::sync::CancellationToken::new();

                init_rpc_api(
                    &opts.node_opts,
                    &opts,
                    peer_table.clone(),
                    local_p2p_node.clone(),
                    local_node_record.lock().await.clone(),
                    store.clone(),
                    blockchain.clone(),
                    cancel_token.clone(),
                    tracker.clone(),
                    rollup_store.clone(),
                )
                .await;

                // Initialize metrics if enabled
                if opts.node_opts.metrics_enabled {
                    init_metrics(&opts.node_opts, tracker.clone());
                }

                if opts.node_opts.p2p_enabled {
                    init_network(
                        &opts.node_opts,
                        &network,
                        &data_dir,
                        local_p2p_node,
                        local_node_record.clone(),
                        signer,
                        peer_table.clone(),
                        store.clone(),
                        tracker.clone(),
                        blockchain.clone(),
                    )
                    .await;
                } else {
                    info!("P2P is disabled");
                }

                let l2_sequencer_cfg = SequencerConfig::from(opts.sequencer_opts);

                let l2_sequencer = ethrex_l2::start_l2(
                    store,
                    rollup_store,
                    blockchain,
                    l2_sequencer_cfg,
                    #[cfg(feature = "metrics")]
                    format!(
                        "http://{}:{}",
                        opts.node_opts.http_addr, opts.node_opts.http_port
                    ),
                )
                .into_future();

                tracker.spawn(l2_sequencer);

                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        info!("Server shut down started...");
                        let node_config_path = PathBuf::from(data_dir + "/node_config.json");
                        info!("Storing config at {:?}...", node_config_path);
                        cancel_token.cancel();
                        let node_config = NodeConfigFile::new(peer_table, local_node_record.lock().await.clone()).await;
                        store_node_config_file(node_config, node_config_path).await;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        info!("Server shutting down!");
                    }
                }
            }
            Self::RemoveDB { datadir, force } => {
                Box::pin(async {
                    ethrex_cli::Subcommand::RemoveDB { datadir, force }
                        .run(&NodeOptions::default()) // This is not used by the RemoveDB command.
                        .await
                })
                .await?
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
                            event_signature,
                        )
                        .await?;

                    if !logs.is_empty() {
                        // Get parent beacon block root hash from block
                        let block = eth_client
                            .get_block_by_number(BlockByNumber::Number(current_block.as_u64()))
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
                                data_dir.join(format!("{}-{}.blob", target_slot, blob.index));
                            std::fs::write(blob_path, blob.blob)?;
                        }

                        println!("Saved blobs for slot {}", target_slot);
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
                        use ethrex_common::H256;

                        // Init stores
                        let store = Store::new_from_genesis(
                            store_path.to_str().expect("Invalid store path"),
                            EngineType::Libmdbx,
                            genesis.to_str().expect("Invalid genesis path"),
                        )
                        .await?;
                        let rollup_store = StoreRollup::new(
                            store_path
                                .join("./rollup_store")
                                .to_str()
                                .expect("Invalid store path"),
                            EngineTypeRollup::Libmdbx,
                        )?;
                        rollup_store
                            .init()
                            .await
                            .expect("Failed to init rollup store");

                        // Get genesis
                        let genesis_header = store.get_block_header(0)?.expect("Genesis block not found");
                        let genesis_block_hash = genesis_header.hash();

                        let mut new_trie = store
                            .state_trie(genesis_block_hash)?
                            .expect("Cannot open state trie");

                        let mut last_block_number = 0;

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
                                .expect("Error applying account updates");

                            let (new_state_root, state_updates, accounts_updates) =
                                (
                                    account_updates_list.state_trie_hash,
                                    account_updates_list.state_updates,
                                    account_updates_list.storage_updates
                                );

                            let pseudo_update_batch = UpdateBatch {
                                account_updates: state_updates,
                                storage_updates: accounts_updates,
                                blocks: vec![],
                                receipts: vec![],
                            };

                            store.store_block_updates(pseudo_update_batch).await.expect("Error storing trie updates");

                            new_trie = store.open_state_trie(new_state_root).expect("Error opening new state trie");

                            // Get withdrawal hashes
                            let withdrawal_hashes = state_diff
                                .withdrawal_logs
                                .iter()
                                .map(|w| {
                                    keccak_hash::keccak(
                                        [
                                            w.address.as_bytes(),
                                            &w.amount.to_big_endian(),
                                            w.tx_hash.as_bytes(),
                                        ]
                                        .concat(),
                                    )
                                })
                                .collect();

                            // Get the first block of the batch
                            let first_block_number = last_block_number + 1;

                            // Build the header of the last block.
                            // Note that its state_root is the root of new_trie.
                            let new_block = BlockHeader {
                                coinbase,
                                state_root: new_trie.hash().expect("Error committing state"),
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
                            store
                                .set_canonical_block(state_diff.last_header.number, new_block_hash)
                                .await?;
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
                                deposit_logs_hash: H256::zero(),
                                withdrawal_hashes,
                                blobs_bundle: BlobsBundle::empty(),
                            };

                            // Store batch info in L2 storage
                            rollup_store
                                .store_batch(batch)
                                .await
                                .expect("Error storing batch");
                        }
                        store.update_latest_block_number(last_block_number).await?;
                    } else {
                        return Err(eyre::eyre!(
                            "Reconstruction is only supported with the libmdbx feature enabled."
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
