use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{
        get_local_p2p_node, get_network, get_signer, init_blockchain, init_metrics, init_rpc_api,
        init_store, init_tracing,
    },
    utils::{set_datadir, store_known_peers},
};
use ethrex_p2p::network::peer_table;
use std::{path::PathBuf, time::Duration};
use tokio_util::task::TaskTracker;
use tracing::info;

#[cfg(any(feature = "l2", feature = "based"))]
use ethrex::l2::L2Options;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let CLI { opts, command } = CLI::parse();

    init_tracing(&opts);

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let data_dir = set_datadir(&opts.datadir);
    let network = get_network(&opts)?;
    let store = init_store(&data_dir, &network)?;
    let blockchain = init_blockchain(opts.evm, store.clone())?;
    let signer = get_signer(&data_dir)?;
    let local_p2p_node = get_local_p2p_node(&opts, &signer)?;
    let tracker = TaskTracker::new();
    let cancel_token = tokio_util::sync::CancellationToken::new();

    let peer_table = peer_table(signer.clone());

    init_rpc_api(
        &opts,
        #[cfg(any(feature = "l2", feature = "based"))]
        &L2Options::default(),
        &signer,
        peer_table.clone(),
        local_p2p_node,
        store.clone(),
        blockchain.clone(),
        cancel_token.clone(),
        tracker.clone(),
    );

    init_metrics(&opts, tracker.clone());

    cfg_if::cfg_if! {
        if #[cfg(feature = "dev")] {
            use ethrex::initializers::init_dev_network;
            init_dev_network(&opts, &store, tracker.clone());
        } else {
            use ethrex::initializers::init_network;
            if opts.p2p_enabled {
                init_network(
                    &opts,
                    &network,
                    &data_dir,
                    local_p2p_node,
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
            let peers_file = PathBuf::from(data_dir + "/peers.json");
            info!("Storing known peers at {:?}...", peers_file);
            cancel_token.cancel();
            store_known_peers(peer_table, peers_file).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            info!("Server shutting down!");
        }
    }

    Ok(())
}
