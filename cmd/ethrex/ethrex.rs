use clap::Parser;
use ethrex::{
    cli::{CLI, Options},
    initializers::{
        get_local_node_record, get_local_p2p_node, get_network, get_signer, init_blockchain,
        init_l1, init_metrics, init_rpc_api, init_store, init_tracing,
    },
    utils::{NodeConfigFile, set_datadir, store_node_config_file},
};
use ethrex_blockchain::BlockchainType;
use ethrex_p2p::{kademlia::KademliaTable, network::peer_table, types::NodeRecord};
#[cfg(feature = "sync-test")]
use ethrex_storage::Store;
#[cfg(feature = "sync-test")]
use std::env;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::Mutex,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::info;

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

async fn server_shutdown(
    data_dir: String,
    cancel_token: &CancellationToken,
    peer_table: Arc<Mutex<KademliaTable>>,
    local_node_record: Arc<Mutex<NodeRecord>>,
) {
    info!("Server shut down started...");
    let node_config_path = PathBuf::from(data_dir + "/node_config.json");
    info!("Storing config at {:?}...", node_config_path);
    cancel_token.cancel();
    let node_config = NodeConfigFile::new(peer_table, local_node_record.lock().await.clone()).await;
    store_node_config_file(node_config, node_config_path).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    info!("Server shutting down!");
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let CLI { opts, command } = CLI::parse();

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    init_tracing(&opts);

    let (data_dir, cancel_token, peer_table, local_node_record) = init_l1(opts).await?;
    let mut signal_terminate = signal(SignalKind::terminate())?;

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            server_shutdown(data_dir, &cancel_token, peer_table, local_node_record).await;
        }
        _ = signal_terminate.recv() => {
            server_shutdown(data_dir, &cancel_token, peer_table, local_node_record).await;
        }
    }

    Ok(())
}
