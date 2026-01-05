use crate::cli::Options as L1Options;
use crate::initializers::{
    self, get_authrpc_socket_addr, get_http_socket_addr, get_local_node_record, get_local_p2p_node,
    get_network, get_signer, init_blockchain, init_network, init_store,
};
use crate::l2::{L2Options, SequencerOptions};
use crate::utils::{
    NodeConfigFile, get_client_version, init_datadir, read_jwtsecret_file, store_node_config_file,
};
use ethrex_blockchain::{Blockchain, BlockchainType, L2Config};
use ethrex_common::fd_limit::raise_fd_limit;
use ethrex_common::types::fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig};
use ethrex_common::{Address, types::DEFAULT_BUILDER_GAS_CEIL};
use ethrex_l2::sequencer::block_producer;
use ethrex_l2::sequencer::l1_committer::{self, regenerate_state};
use ethrex_p2p::{
    discv4::peer_table::PeerTable,
    network::P2PContext,
    peer_handler::PeerHandler,
    rlpx::{initiator::RLPxInitiator, l2::l2_connection::P2PBasedContext},
    sync_manager::SyncManager,
    types::{Node, NodeRecord},
};
use ethrex_storage::Store;
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use eyre::OptionExt;
use secp256k1::SecretKey;
use spawned_concurrency::tasks::GenServerHandle;
use std::{fs::read_to_string, path::Path, sync::Arc, time::Duration};
use tokio::task::JoinSet;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt, reload};
use tui_logger::{LevelFilter, TuiTracingSubscriberLayer};
use url::Url;

#[allow(clippy::too_many_arguments)]
fn init_rpc_api(
    opts: &L1Options,
    l2_opts: &L2Options,
    peer_handler: Option<PeerHandler>,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    store: Store,
    blockchain: Arc<Blockchain>,
    syncer: Option<Arc<SyncManager>>,
    tracker: TaskTracker,
    rollup_store: StoreRollup,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    gas_ceil: Option<u64>,
) {
    init_datadir(&opts.datadir);

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
        log_filter_handler,
        gas_ceil.unwrap_or(DEFAULT_BUILDER_GAS_CEIL),
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

pub async fn init_rollup_store(datadir: &Path) -> StoreRollup {
    #[cfg(feature = "l2-sql")]
    let engine_type = EngineTypeRollup::SQL;
    #[cfg(not(feature = "l2-sql"))]
    let engine_type = EngineTypeRollup::InMemory;
    let rollup_store =
        StoreRollup::new(datadir, engine_type).expect("Failed to create StoreRollup");
    rollup_store
        .init()
        .await
        .expect("Failed to init rollup store");
    rollup_store
}

fn init_metrics(opts: &L1Options, network: &str, tracker: TaskTracker) {
    // Initialize node version metrics
    ethrex_metrics::node::MetricsNode::init(
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_RUSTC_SEMVER"),
        env!("VERGEN_RUSTC_HOST_TRIPLE"),
        network,
    );

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

pub fn init_tracing(
    opts: &L2Options,
) -> (
    Option<reload::Handle<EnvFilter, Registry>>,
    Option<tracing_appender::non_blocking::WorkerGuard>,
) {
    if !opts.sequencer_opts.no_monitor {
        let level_filter = EnvFilter::builder()
            .parse_lossy("debug,tower_http::trace=debug,reqwest_tracing=off,hyper=off,libsql=off,ethrex::initializers=off,ethrex::l2::initializers=off,ethrex::l2::command=off");
        let subscriber = tracing_subscriber::registry()
            .with(TuiTracingSubscriberLayer)
            .with(level_filter);
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
        tui_logger::init_logger(LevelFilter::max()).expect("Failed to initialize tui_logger");

        // Monitor already registers all log levels
        (None, None)
    } else {
        let (handle, guard) = initializers::init_tracing(&opts.node_opts);
        (Some(handle), guard)
    }
}

async fn shutdown_sequencer_handles(
    committer_handle: Option<GenServerHandle<l1_committer::L1Committer>>,
    block_producer_handle: Option<GenServerHandle<block_producer::BlockProducer>>,
) {
    // These GenServers run via start_blocking, so aborting the JoinSet alone never stops them.
    // Sending Abort elicits CastResponse::Stop and lets the blocking loop unwind cleanly.
    if let Some(mut handle) = committer_handle {
        handle
            .cast(l1_committer::InMessage::Abort)
            .await
            .inspect_err(|err| warn!("Failed to send committer abort: {err:?}"))
            .ok();
    }
    if let Some(mut handle) = block_producer_handle {
        handle
            .cast(block_producer::InMessage::Abort)
            .await
            .inspect_err(|err| warn!("Failed to send block producer abort: {err:?}"))
            .ok();
    }
}

pub async fn init_l2(
    opts: L2Options,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
) -> eyre::Result<()> {
    raise_fd_limit()?;
    let datadir = opts.node_opts.datadir.clone();
    init_datadir(&opts.node_opts.datadir);

    let rollup_store_dir = datadir.join("rollup_store");

    // Checkpoints are stored in the main datadir
    let checkpoints_dir = datadir.clone();

    let network = get_network(&opts.node_opts);

    let genesis = network.get_genesis()?;
    let store = init_store(&datadir, genesis.clone()).await?;
    let rollup_store = init_rollup_store(&rollup_store_dir).await;

    let operator_fee_config = get_operator_fee_config(&opts.sequencer_opts)?;
    let l1_fee_config = get_l1_fee_config(&opts.sequencer_opts);

    let fee_config = FeeConfig {
        base_fee_vault: opts
            .sequencer_opts
            .block_producer_opts
            .base_fee_vault_address,
        operator_fee_config,
        l1_fee_config,
    };

    // We wrap fee_config in an Arc<RwLock> to let the watcher
    // update the L1 fee periodically.
    let l2_config = L2Config {
        fee_config: Arc::new(std::sync::RwLock::new(fee_config)),
    };

    let blockchain_opts = ethrex_blockchain::BlockchainOptions {
        max_mempool_size: opts.node_opts.mempool_max_size,
        r#type: BlockchainType::L2(l2_config),
        perf_logs_enabled: true,
    };

    let blockchain = init_blockchain(store.clone(), blockchain_opts.clone());

    regenerate_state(&store, &rollup_store, &blockchain, None).await?;

    let signer = get_signer(&datadir);

    let local_p2p_node = get_local_p2p_node(&opts.node_opts, &signer);

    let local_node_record = get_local_node_record(&datadir, &local_p2p_node, &signer);

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();
    let mut join_set = JoinSet::new();

    let cancel_token = tokio_util::sync::CancellationToken::new();

    let (peer_handler, syncer) = if !opts.node_opts.p2p_disabled {
        if !opts.sequencer_opts.based {
            blockchain.set_synced();
        }
        let peer_table = PeerTable::spawn(opts.node_opts.target_peers);
        let p2p_context = P2PContext::new(
            local_p2p_node.clone(),
            tracker.clone(),
            signer,
            peer_table.clone(),
            store.clone(),
            blockchain.clone(),
            get_client_version(),
            #[cfg(feature = "l2")]
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
            opts.node_opts.tx_broadcasting_time_interval,
            opts.node_opts.lookup_interval,
        )
        .expect("P2P context could not be created");
        let initiator = RLPxInitiator::spawn(p2p_context.clone()).await;
        let peer_handler = PeerHandler::new(peer_table, initiator);

        // Create SyncManager
        let syncer = SyncManager::new(
            peer_handler.clone(),
            &opts.node_opts.syncmode,
            cancel_token.clone(),
            blockchain.clone(),
            store.clone(),
            opts.node_opts.datadir.clone(),
        )
        .await;

        // TODO: This should be handled differently, the current problem
        // with using opts.node_opts.p2p_disabled is that with the removal
        // of the l2 feature flag, p2p_disabled is set to false by default
        // prioritizing the L1 UX.
        init_network(
            &opts.node_opts,
            &network,
            &datadir,
            peer_handler.clone(),
            tracker.clone(),
            blockchain.clone(),
            p2p_context,
        )
        .await;
        (Some(peer_handler), Some(Arc::new(syncer)))
    } else {
        (None, None)
    };

    init_rpc_api(
        &opts.node_opts,
        &opts,
        peer_handler.clone(),
        local_p2p_node.clone(),
        local_node_record.clone(),
        store.clone(),
        blockchain.clone(),
        syncer,
        tracker.clone(),
        rollup_store.clone(),
        log_filter_handler,
        Some(opts.sequencer_opts.block_producer_opts.block_gas_limit),
    );

    // Initialize metrics if enabled
    if opts.node_opts.metrics_enabled {
        init_metrics(&opts.node_opts, &network.to_string(), tracker);
    }

    let sequencer_cancellation_token = CancellationToken::new();
    let l2_url = Url::parse(&format!(
        "http://{}:{}",
        opts.node_opts.http_addr, opts.node_opts.http_port
    ))
    .map_err(|err| eyre::eyre!("Failed to parse L2 RPC URL: {err}"))?;
    let (committer_handle, block_producer_handle, l2_sequencer) = ethrex_l2::start_l2(
        store,
        rollup_store,
        blockchain,
        opts.sequencer_opts.try_into()?,
        sequencer_cancellation_token.clone(),
        l2_url,
        genesis,
        checkpoints_dir,
    )
    .await?;
    join_set.spawn(l2_sequencer);

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            shutdown_sequencer_handles(
                committer_handle.clone(),
                block_producer_handle.clone()
            ).await;
            join_set.abort_all();
        }
        _ = sequencer_cancellation_token.cancelled() => {
            shutdown_sequencer_handles(committer_handle.clone(), block_producer_handle.clone()).await;
        }
    }
    info!("Server shut down started...");
    let node_config_path = datadir.join("node_config.json");
    info!(path = %node_config_path.display(), "Storing node config");
    cancel_token.cancel();
    if !opts.node_opts.p2p_disabled {
        let peer_handler = peer_handler.ok_or_eyre("Peer handler not initialized")?;
        let node_config = NodeConfigFile::new(peer_handler.peer_table, local_node_record).await;
        store_node_config_file(node_config, node_config_path);
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    info!("Server shutting down!");
    Ok(())
}

pub fn get_l1_fee_config(sequencer_opts: &SequencerOptions) -> Option<L1FeeConfig> {
    if sequencer_opts.based {
        // If based is enabled, skip L1 fee configuration
        return None;
    }

    sequencer_opts
        .block_producer_opts
        .l1_fee_vault_address
        .map(|addr| L1FeeConfig {
            l1_fee_vault: addr,
            l1_fee_per_blob_gas: 0, // This is set by the L1 watcher
        })
}

pub fn get_operator_fee_config(
    sequencer_opts: &SequencerOptions,
) -> eyre::Result<Option<OperatorFeeConfig>> {
    if sequencer_opts.based {
        // If based is enabled, skip operator fee configuration
        return Ok(None);
    }

    let fee = sequencer_opts.block_producer_opts.operator_fee_per_gas;

    let address = sequencer_opts
        .block_producer_opts
        .operator_fee_vault_address;

    let operator_fee_config =
        if let (Some(operator_fee_vault), Some(operator_fee_per_gas)) = (address, fee) {
            Some(OperatorFeeConfig {
                operator_fee_vault,
                operator_fee_per_gas,
            })
        } else {
            None
        };
    Ok(operator_fee_config)
}
