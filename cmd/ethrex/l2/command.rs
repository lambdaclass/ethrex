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
use ethrex_common::{Address, U256};
use ethrex_l2::SequencerConfig;
use ethrex_p2p::network::peer_table;
use ethrex_rpc::{
    clients::{beacon::BeaconClient, eth::BlockByNumber},
    EthClient,
};
use eyre::OptionExt;
use keccak_hash::keccak;
use reqwest::Url;
use std::{fs::create_dir_all, future::IntoFuture, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::info;

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum Command {
    #[clap(about = "Initialize an ethrex L2 node", visible_alias = "i")]
    Init {
        #[command(flatten)]
        opts: Options,
    },
    #[clap(name = "removedb", about = "Remove the database", visible_aliases = ["rm", "clean"])]
    RemoveDB {
        #[arg(long = "datadir", value_name = "DATABASE_DIRECTORY", default_value = DEFAULT_L2_DATADIR, required = false)]
        datadir: String,
        #[clap(long = "force", required = false, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    #[clap(about = "Launch a server that listens for Blobs submissions and saves them offline.")]
    BlobsSaver {
        #[clap(
            short = 'c',
            long = "contract",
            help = "The contract address to listen to."
        )]
        contract_address: Address,
        #[clap(short = 'd', long, help = "The directory to save the blobs.")]
        data_dir: PathBuf,
        #[clap(short = 'e', long)]
        l1_eth_rpc: Url,
        #[clap(short = 'b', long)]
        l1_beacon_rpc: Url,
    },
}

impl Command {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            Command::Init { opts } => {
                let data_dir = set_datadir(&opts.node_opts.datadir);
                let rollup_store_dir = data_dir.clone() + "/rollup_store";

                let network = get_network(&opts.node_opts);

                let store = init_store(&data_dir, &network).await;
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

                let l2_sequencer =
                    ethrex_l2::start_l2(store, rollup_store, blockchain, l2_sequencer_cfg)
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
        }
        Ok(())
    }
}
