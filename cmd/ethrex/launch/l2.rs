use crate::launch::{
    common::{
        get_local_p2p_node, get_network, get_signer, init_blockchain, init_metrics, init_network,
        init_rpc_api, init_store, init_tracing,
    },
    DEFAULT_DATADIR,
};
use crate::utils::{set_datadir, store_known_peers};
use ethrex_p2p::network::peer_table;
use std::path::PathBuf;
use std::{future::IntoFuture, path::Path, time::Duration};
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};

pub async fn launch(matches: clap::ArgMatches) {
    let data_dir = matches
        .get_one::<String>("datadir")
        .map_or(set_datadir(DEFAULT_DATADIR), |datadir| set_datadir(datadir));

    if matches.subcommand_matches("removedb").is_some() {
        let path = Path::new(&data_dir);
        if path.exists() {
            std::fs::remove_dir_all(path).expect("Failed to remove data directory");
        } else {
            warn!("Data directory does not exist: {}", data_dir);
        }
        return;
    }

    let dev_mode = *matches.get_one::<bool>("dev").unwrap_or(&false);
    // We do not want to start the networking module if the l2 feature is enabled.
    if dev_mode {
        error!("Cannot run with DEV_MODE if the `l2` feature is enabled.");
        panic!("Run without the --dev argument.");
    }

    init_tracing(&matches);

    let network = get_network(&matches);

    let store = init_store(&data_dir, &network);

    let blockchain = init_blockchain(&matches, store.clone());

    let signer = get_signer(&data_dir);

    let local_p2p_node = get_local_p2p_node(&matches, &signer);

    let peer_table = peer_table(signer.clone());

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();

    let cancel_token = tokio_util::sync::CancellationToken::new();

    init_rpc_api(
        &matches,
        &signer,
        peer_table.clone(),
        local_p2p_node,
        store.clone(),
        blockchain.clone(),
        cancel_token.clone(),
        tracker.clone(),
    );

    init_metrics(&matches, tracker.clone());

    init_network(
        &matches,
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

    let l2_proposer = ethrex_l2::start_proposer(store, blockchain).into_future();

    tracker.spawn(l2_proposer);

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
}
