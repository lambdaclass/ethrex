use std::fs::read_to_string;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ethrex_blockchain::{Blockchain, BlockchainType};
use ethrex_common::Address;
use ethrex_l2::SequencerConfig;
use ethrex_p2p::kademlia::Kademlia;
use ethrex_p2p::network::peer_table;
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::rlpx::l2::l2_connection::P2PBasedContext;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_p2p::types::{Node, NodeRecord};
use ethrex_storage::Store;
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use secp256k1::SecretKey;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tui_logger::{LevelFilter, TuiTracingSubscriberLayer};

use crate::cli::Options as L1Options;
use crate::initializers::{
    self, get_authrpc_socket_addr, get_http_socket_addr, get_local_node_record, get_local_p2p_node,
    get_network, get_signer, init_blockchain, init_network, init_store,
};
use crate::l2::L2Options;
use crate::utils::{
    NodeConfigFile, get_client_version, init_datadir, read_jwtsecret_file, store_node_config_file,
};

#[allow(clippy::too_many_arguments)]
async fn init_rpc_api(
    opts: &L1Options,
    l2_opts: &L2Options,
    peer_table: Kademlia,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    store: Store,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    tracker: TaskTracker,
    rollup_store: StoreRollup,
) {
    let peer_handler = PeerHandler::new(peer_table);

    // Create SyncManager
    let syncer = SyncManager::new(
        peer_handler.clone(),
        opts.syncmode.clone(),
        cancel_token,
        blockchain.clone(),
        store.clone(),
        init_datadir(&opts.datadir),
    )
    .await;

    let rpc_api = ethrex_l2_rpc::start_api(
        get_http_socket_addr(opts),
        get_authrpc_socket_addr(opts),
        store,
        blockchain,
        read_jwtsecret_file(&opts.authrpc_jwtsecret),
        local_p2p_node,
        local_node_record,
        syncer,
        peer_handler,
        get_client_version(),
        get_valid_delegation_addresses(l2_opts),
        l2_opts.sponsor_private_key,
        rollup_store,
    );

    tracker.spawn(rpc_api);
}

fn get_valid_delegation_addresses(l2_opts: &L2Options) -> Vec<Address> {
    let Some(ref path) = l2_opts.sponsorable_addresses_file_path else {
        warn!("No valid addresses provided, ethrex_SendTransaction will always fail");
        return Vec::new();
    };
    let addresses: Vec<Address> = read_to_string(path)
        .unwrap_or_else(|_| panic!("Failed to load file {path}"))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string().parse::<Address>())
        .filter_map(Result::ok)
        .collect();
    if addresses.is_empty() {
        warn!("No valid addresses provided, ethrex_SendTransaction will always fail");
    }
    addresses
}

pub async fn init_rollup_store(data_dir: &str) -> StoreRollup {
    cfg_if::cfg_if! {
        if #[cfg(feature = "rollup_storage_sql")] {
            let engine_type = EngineTypeRollup::SQL;
        }
        else {
            let engine_type = EngineTypeRollup::InMemory;
        }
    }
    let rollup_store =
        StoreRollup::new(data_dir, engine_type).expect("Failed to create StoreRollup");
    rollup_store
        .init()
        .await
        .expect("Failed to init rollup store");
    rollup_store
}

fn init_metrics(opts: &L1Options, tracker: TaskTracker) {
    tracing::info!(
        "Starting metrics server on {}:{}",
        opts.metrics_addr,
        opts.metrics_port
    );
    let metrics_api = ethrex_metrics::l2::api::start_prometheus_metrics_api(
        opts.metrics_addr.clone(),
        opts.metrics_port.clone(),
    );
    tracker.spawn(metrics_api);
}

pub fn init_tracing(opts: &L2Options) {
    if !opts.sequencer_opts.no_monitor {
        let level_filter = EnvFilter::builder()
            .parse_lossy("debug,tower_http::trace=debug,reqwest_tracing=off,hyper=off,libsql=off,ethrex::initializers=off,ethrex::l2::initializers=off,ethrex::l2::command=off");
        let subscriber = tracing_subscriber::registry()
            .with(TuiTracingSubscriberLayer)
            .with(level_filter);
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
        tui_logger::init_logger(LevelFilter::max()).expect("Failed to initialize tui_logger");
    } else {
        initializers::init_tracing(&opts.node_opts);
    }
}

pub async fn init_l2(opts: L2Options) -> eyre::Result<()> {
    let data_dir = init_datadir(&opts.node_opts.datadir);
    let rollup_store_dir = data_dir.clone() + "/rollup_store";

    let network = get_network(&opts.node_opts);

    let genesis = network.get_genesis()?;
    let store = init_store(&data_dir, genesis).await;
    let rollup_store = init_rollup_store(&rollup_store_dir).await;

    let blockchain = init_blockchain(store.clone(), BlockchainType::L2, true);

    let signer = get_signer(&data_dir);

    let local_p2p_node = get_local_p2p_node(&opts.node_opts, &signer);

    let local_node_record = Arc::new(Mutex::new(get_local_node_record(
        &data_dir,
        &local_p2p_node,
        &signer,
    )));

    let peer_table = peer_table();

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();
    let mut join_set = JoinSet::new();

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

    let l2_sequencer_cfg = SequencerConfig::try_from(opts.sequencer_opts).inspect_err(|err| {
        error!("{err}");
    })?;
    let cancellation_token = CancellationToken::new();

    // TODO: This should be handled differently, the current problem
    // with using opts.node_opts.p2p_enabled is that with the removal
    // of the l2 feature flag, p2p_enabled is set to true by default
    // prioritizing the L1 UX.
    if l2_sequencer_cfg.based.enabled {
        init_network(
            &opts.node_opts,
            &network,
            &data_dir,
            local_p2p_node,
            local_node_record.clone(),
            signer,
            peer_table.clone(),
            store.clone(),
            tracker,
            blockchain.clone(),
            Some(P2PBasedContext {
                store_rollup: rollup_store.clone(),
                // TODO: The Web3Signer refactor introduced a limitation where the committer key cannot be accessed directly because the signer could be either Local or Remote.
                // The Signer enum cannot be used in the P2PBasedContext struct due to cyclic dependencies between the l2-rpc and p2p crates.
                // As a temporary solution, a dummy committer key is used until a proper mechanism to utilize the Signer enum is implemented.
                // This should be replaced with the Signer enum once the refactor is complete.
                committer_key: Arc::new(
                    SecretKey::from_slice(
                        &hex::decode(
                            "385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924",
                        )
                        .expect("Invalid committer key"),
                    )
                    .expect("Failed to create committer key"),
                ),
            }),
        )
        .await;
    } else {
        info!("P2P is disabled");
    }

    let l2_sequencer = ethrex_l2::start_l2(
        store,
        rollup_store,
        blockchain,
        l2_sequencer_cfg,
        cancellation_token.clone(),
        #[cfg(feature = "metrics")]
        format!(
            "http://{}:{}",
            opts.node_opts.http_addr, opts.node_opts.http_port
        ),
    )
    .into_future();

    join_set.spawn(l2_sequencer);

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            join_set.abort_all();
        }
        _ = cancellation_token.cancelled() => {
        }
    }
    info!("Server shut down started...");
    let node_config_path = PathBuf::from(data_dir + "/node_config.json");
    info!(path = %node_config_path.display(), "Storing node config");
    cancel_token.cancel();
    let node_config = NodeConfigFile::new(peer_table, local_node_record.lock().await.clone()).await;
    store_node_config_file(node_config, node_config_path).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    info!("Server shutting down!");
    Ok(())
}
