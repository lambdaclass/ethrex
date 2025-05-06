use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{
        get_local_node_record, get_local_p2p_node, get_network, get_signer, init_blockchain,
        init_metrics, init_rpc_api, init_store, init_tracing,
    },
    utils::{set_datadir, store_config_file, ConfigFile},
};
use ethrex_p2p::network::peer_table;
use std::{path::PathBuf, time::Duration};
use tokio_util::task::TaskTracker;
use tracing::info;

#[cfg(any(feature = "l2", feature = "based"))]
use ethrex::l2::L2Options;
#[cfg(feature = "l2")]
use ethrex_storage_rollup::StoreRollup;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let CLI { opts, command } = CLI::parse();

    init_tracing(&opts);

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let data_dir = set_datadir(&opts.datadir);

    let network = get_network(&opts);

    let store = init_store(&data_dir, &network).await;

    let blockchain = init_blockchain(opts.evm, store.clone());

    let signer = get_signer(&data_dir);

    let local_p2p_node = get_local_p2p_node(&opts, &signer);

    let local_node_record = get_local_node_record(&data_dir, &local_p2p_node, &signer);

    let peer_table = peer_table(signer.clone());

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();

    let cancel_token = tokio_util::sync::CancellationToken::new();

    init_rpc_api(
        &opts,
        #[cfg(any(feature = "l2", feature = "based"))]
        &L2Options::default(),
        peer_table.clone(),
        local_p2p_node,
        local_node_record.clone(),
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
            let config_path = PathBuf::from(data_dir + "/config.json");
            info!("Storing config at {:?}...", config_path);
            cancel_token.cancel();
            let config = ConfigFile::new(peer_table, 0).await;
            store_config_file(config, config_path).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            info!("Server shutting down!");
        }
    }

    Ok(())
}
