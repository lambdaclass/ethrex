use ethrex::{
    initializers::{
        get_local_p2p_node, get_network, get_signer, init_blockchain, init_metrics, init_rpc_api,
        init_store, init_tracing,
    },
    utils::{set_datadir, store_known_peers},
    DEFAULT_DATADIR,
};
use ethrex_p2p::network::peer_table;
use std::{path::PathBuf, time::Duration};
use tokio_util::task::TaskTracker;
use tracing::info;

#[tokio::main]
async fn main() {
    let matches = ethrex::cli::cli().get_matches();

    init_tracing(&matches);

    if let Some(subcommand_matches) = matches.subcommand_matches("removedb") {
        let data_dir = subcommand_matches
            .get_one::<String>("datadir")
            .map_or(set_datadir(DEFAULT_DATADIR), |datadir| set_datadir(datadir));
        ethrex::removedb::remove_db(&data_dir);
        return;
    }

    let evm_engine = matches
        .get_one::<String>("evm")
        .unwrap_or(&"revm".to_string())
        .clone()
        .try_into()
        .unwrap_or_else(|e| panic!("{}", e));

    let network = get_network(&matches);

    let data_dir = matches
        .get_one::<String>("datadir")
        .map_or(set_datadir(DEFAULT_DATADIR), |datadir| set_datadir(datadir));

    if let Some(subcommand_matches) = matches.subcommand_matches("import") {
        ethrex::import::import_blocks_from_path(subcommand_matches, data_dir, evm_engine, &network);
        return;
    }

    let store = init_store(&data_dir, &network);

    let blockchain = init_blockchain(evm_engine, store.clone());

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

    cfg_if::cfg_if! {
        if #[cfg(feature = "dev")] {
            use ethrex::initializers::init_dev_network;

            init_dev_network(&matches, &store, tracker.clone());
        } else {
            use ethrex::initializers::{init_network};

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
        }
    }

    cfg_if::cfg_if! {
        if #[cfg(all(feature = "l2", not(feature = "dev")))] {
            use std::future::IntoFuture;

            let l2_proposer = ethrex_l2::start_proposer(store, blockchain).into_future();

            tracker.spawn(l2_proposer);
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
}
