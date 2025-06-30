use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{
        get_local_node_record, get_local_p2p_node, get_network, get_signer, init_blockchain,
        init_metrics, init_rpc_api, init_store, init_tracing,
    },
    utils::{NodeConfigFile, set_datadir, store_node_config_file},
};
#[cfg(feature = "l2")]
use ethrex_blockchain::sequencer_state::{SequencerState, SequencerStatus};
use ethrex_p2p::network::peer_table;
#[cfg(feature = "sync-test")]
use ethrex_storage::Store;
#[cfg(feature = "sync-test")]
use std::env;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::info;

#[cfg(feature = "l2")]
use ethrex::l2::L2Options;
#[cfg(feature = "l2")]
use ethrex_storage_rollup::StoreRollup;
#[cfg(feature = "sync-test")]
async fn set_sync_block(store: &Store) {
    if let Ok(block_number) = env::var("SYNC_BLOCK_NUM") {
        let block_number = block_number
            .parse()
            .expect("Block number provided by environment is not numeric");
        let block_hash = store
            .get_canonical_block_hash(block_number)
            .await
            .expect("Could not get hash for block number provided by env variable")
            .expect("Could not get hash for block number provided by env variable");
        store
            .update_latest_block_number(block_number)
            .await
            .expect("Failed to update latest block number");
        store
            .set_canonical_block(block_number, block_hash)
            .await
            .expect("Failed to set latest canonical block");
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let CLI { opts, command } = CLI::parse();

    init_tracing(&opts);

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let data_dir = set_datadir(&opts.datadir);

    let network = get_network(&opts);

    let genesis = network.get_genesis();
    let store = init_store(&data_dir, genesis).await;

    #[cfg(feature = "sync-test")]
    set_sync_block(&store).await;

    let blockchain = init_blockchain(opts.evm, store.clone());

    let signer = get_signer(&data_dir);

    let local_p2p_node = get_local_p2p_node(&opts, &signer);

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
        &opts,
        #[cfg(feature = "l2")]
        &L2Options::default(),
        peer_table.clone(),
        local_p2p_node.clone(),
        local_node_record.lock().await.clone(),
        store.clone(),
        blockchain.clone(),
        cancel_token.clone(),
        tracker.clone(),
        #[cfg(feature = "l2")]
        StoreRollup::default(),
    )
    .await;

    if opts.metrics_enabled {
        init_metrics(&opts, tracker.clone());
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "dev")] {
            use ethrex::initializers::init_dev_network;

            init_dev_network(&opts, &store, tracker.clone()).await;
        } else {
            use ethrex::initializers::init_network;

            if opts.p2p_enabled {
                init_network(
                    &opts,
                    &network,
                    &data_dir,
                    local_p2p_node,
                    local_node_record.clone(),
                    signer,
                    peer_table.clone(),
                    store.clone(),
                    tracker.clone(),
                    blockchain.clone(),
                    false,
                    #[cfg(feature = "l2")]
                    StoreRollup::default(),
                    None,
                    #[cfg(feature = "l2")]
                    SequencerState::from(SequencerStatus::default()),
                )
                .await;
            } else {
                info!("P2P is disabled");
            }
        }
    }

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

    Ok(())
}
