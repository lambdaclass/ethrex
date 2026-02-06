use crate::{
    cli::{LogColor, Options},
    utils::{
        display_chain_initialization, get_client_version, init_datadir, is_memory_datadir,
        parse_socket_addr, read_jwtsecret_file, read_node_config_file,
    },
};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType};
use ethrex_common::fd_limit::raise_fd_limit;
use ethrex_common::types::Genesis;
use ethrex_config::networks::Network;

use ethrex_metrics::profiling::{FunctionProfilingLayer, initialize_block_processing_profile};
use ethrex_metrics::rpc::initialize_rpc_metrics;
use ethrex_p2p::rlpx::initiator::RLPxInitiator;
use ethrex_p2p::{
    network::P2PContext,
    peer_handler::PeerHandler,
    peer_table::PeerTable,
    sync::SyncMode,
    sync_manager::SyncManager,
    types::{Node, NodeRecord},
    utils::public_key_from_signing_key,
};
use ethrex_storage::{EngineType, Store, error::StoreError};
use local_ip_address::{local_ip, local_ipv6};
use rand::rngs::OsRng;
use secp256k1::SecretKey;
#[cfg(feature = "sync-test")]
use std::env;
use std::{
    fs,
    io::IsTerminal,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
#[cfg(not(feature = "l2"))]
use tracing::error;
use tracing::{Level, debug, info, warn};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, filter::Directive, fmt, layer::SubscriberExt, reload,
};

// Compile-time check to ensure that at least one of the database features is enabled.
#[cfg(not(feature = "rocksdb"))]
const _: () = {
    compile_error!("Database feature must be enabled (Available: `rocksdb`).");
};

pub fn init_tracing(
    opts: &Options,
) -> (
    reload::Handle<EnvFilter, Registry>,
    Option<tracing_appender::non_blocking::WorkerGuard>,
) {
    let log_filter = EnvFilter::builder()
        .with_default_directive(Directive::from(opts.log_level))
        .from_env_lossy();

    let (filter, filter_handle) = reload::Layer::new(log_filter);

    let stdout_is_tty = std::io::stdout().is_terminal();
    let use_color = match opts.log_color {
        LogColor::Always => true,
        LogColor::Never => false,
        LogColor::Auto => stdout_is_tty,
    };

    let include_target = matches!(opts.log_level, Level::DEBUG | Level::TRACE);

    let fmt_layer = fmt::layer()
        .with_target(include_target)
        .with_ansi(use_color);

    let (file_layer, guard) = if let Some(log_dir) = &opts.log_dir {
        if !log_dir.exists() {
            std::fs::create_dir_all(log_dir).expect("Failed to create log directory");
        }

        let branch = env!("VERGEN_GIT_BRANCH").replace('/', "-");
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let log_file = log_dir.join(format!("ethrex_{}_{}.log", branch, timestamp));

        let file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(log_file)
            .expect("Failed to open log file");

        let (non_blocking, guard) = tracing_appender::non_blocking(file);
        let file_layer = fmt::layer()
            .with_target(include_target)
            .with_ansi(false)
            .with_writer(non_blocking);
        (Some(file_layer), Some(guard))
    } else {
        (None, None)
    };

    let profiling_layer = opts.metrics_enabled.then_some(FunctionProfilingLayer);

    let subscriber = Registry::default()
        .with(fmt_layer.and_then(file_layer).with_filter(filter))
        .with(profiling_layer);

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    (filter_handle, guard)
}

pub fn init_metrics(opts: &Options, network: &Network, tracker: TaskTracker) {
    // Initialize node version metrics
    ethrex_metrics::node::MetricsNode::init(
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_RUSTC_SEMVER"),
        env!("VERGEN_RUSTC_HOST_TRIPLE"),
        &network.to_string(),
    );

    tracing::info!(
        "Starting metrics server on {}:{}",
        opts.metrics_addr,
        opts.metrics_port
    );
    let metrics_api = ethrex_metrics::api::start_prometheus_metrics_api(
        opts.metrics_addr.clone(),
        opts.metrics_port.clone(),
    );

    initialize_block_processing_profile();
    initialize_rpc_metrics();

    tracker.spawn(metrics_api);
}

/// Opens a new or pre-existing Store and loads the initial state provided by the network
pub async fn init_store(datadir: impl AsRef<Path>, genesis: Genesis) -> Result<Store, StoreError> {
    let mut store = open_store(datadir.as_ref())?;
    store.add_initial_state(genesis).await?;
    Ok(store)
}

/// Initializes a pre-existing Store
pub async fn load_store(datadir: &Path) -> Result<Store, StoreError> {
    let store = open_store(datadir)?;
    store.load_initial_state().await?;
    Ok(store)
}

/// Opens a pre-existing Store or creates a new one
pub fn open_store(datadir: &Path) -> Result<Store, StoreError> {
    if is_memory_datadir(datadir) {
        Store::new(datadir, EngineType::InMemory)
    } else {
        #[cfg(feature = "rocksdb")]
        let engine_type = EngineType::RocksDB;
        #[cfg(feature = "metrics")]
        ethrex_metrics::process::set_datadir_path(datadir.to_path_buf());
        Store::new(datadir, engine_type)
    }
}

pub fn init_blockchain(store: Store, blockchain_opts: BlockchainOptions) -> Arc<Blockchain> {
    info!("Initiating blockchain with levm");
    Blockchain::new(store, blockchain_opts).into()
}

#[expect(clippy::too_many_arguments)]
pub async fn init_rpc_api(
    opts: &Options,
    peer_handler: PeerHandler,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    store: Store,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    tracker: TaskTracker,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
) {
    if !is_memory_datadir(&opts.datadir) {
        init_datadir(&opts.datadir);
    }

    let syncmode = if opts.dev {
        &SyncMode::Full
    } else {
        &opts.syncmode
    };

    // Create SyncManager
    let syncer = SyncManager::new(
        peer_handler.clone(),
        syncmode,
        cancel_token,
        blockchain.clone(),
        store.clone(),
        opts.datadir.clone(),
    )
    .await;

    let ws_socket_opts = if opts.ws_enabled {
        Some(get_ws_socket_addr(opts))
    } else {
        None
    };

    let rpc_api = ethrex_rpc::start_api(
        get_http_socket_addr(opts),
        ws_socket_opts,
        Some(get_authrpc_socket_addr(opts)),
        store,
        blockchain,
        read_jwtsecret_file(&opts.authrpc_jwtsecret),
        local_p2p_node,
        local_node_record,
        syncer,
        peer_handler,
        get_client_version(),
        log_filter_handler,
        opts.gas_limit,
        opts.extra_data.clone(),
        None,
    );

    tracker.spawn(rpc_api);
}

#[allow(clippy::too_many_arguments)]
pub async fn init_network(
    opts: &Options,
    network: &Network,
    datadir: &Path,
    peer_handler: PeerHandler,
    tracker: TaskTracker,
    blockchain: Arc<Blockchain>,
    context: P2PContext,
) {
    #[cfg(not(feature = "l2"))]
    if opts.dev {
        error!("Binary wasn't built with The feature flag `dev` enabled.");
        panic!(
            "Build the binary with the `dev` feature in order to use the `--dev` cli's argument."
        );
    }

    let bootnodes = get_bootnodes(opts, network, datadir);

    ethrex_p2p::start_network(context, bootnodes)
        .await
        .expect("Network starts");

    tracker.spawn(ethrex_p2p::periodically_show_peer_stats(
        blockchain,
        peer_handler.peer_table,
    ));
}

/// Initialize an L1 node in dev mode using the GenServer block builder.
///
/// This replaces the old Engine API-based dev block producer with an on-demand
/// block builder that creates blocks immediately when transactions arrive.
///
/// ## Architecture
///
/// ```text
/// eth_sendRawTransaction
///   │
///   ├─► mempool (existing behavior)
///   │
///   └─► dev_tx_sender channel ──► forwarding task ──► BlockBuilder GenServer
///                                                        │
///                                                        ├─ on-demand: build block immediately
///                                                        └─ interval:  queue, build on timer tick
/// ```
///
/// ## What's skipped vs `init_l1`
///
/// - **Auth RPC server**: No consensus client in dev mode, so `authrpc_addr` is `None`
/// - **P2P networking**: No peer discovery or sync needed
/// - **Consensus-layer timer**: No CL heartbeat monitoring
///
/// ## CLI options
///
/// - `--dev.block-time <ms>`: Build blocks at intervals (default: on-demand)
/// - `--dev.coinbase <address>`: Set the coinbase address (default: zero address)
#[cfg(feature = "dev")]
pub async fn init_l1_dev(
    opts: Options,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
) -> eyre::Result<(PathBuf, CancellationToken, PeerTable, NodeRecord)> {
    info!("Running in DEV_MODE (GenServer block builder)");

    let coinbase = opts
        .dev_coinbase
        .as_ref()
        .map(|s| s.parse::<ethrex_common::Address>())
        .transpose()
        .map_err(|e| eyre::eyre!("Invalid --dev.coinbase address: {e}"))?
        .unwrap_or_default();

    let config = ethrex_dev::BlockBuilderConfig {
        coinbase,
        block_time_ms: opts.dev_block_time,
        gas_ceil: opts.gas_limit,
    };

    let (mut handle, store, blockchain) = ethrex_dev::BlockBuilder::spawn(config)
        .await
        .map_err(|e| eyre::eyre!("Failed to spawn block builder: {e}"))?;

    // Create forwarding channel: RPC → BlockBuilder
    let (tx_sender, mut tx_receiver) =
        tokio::sync::mpsc::unbounded_channel::<(
            Box<ethrex_common::types::Transaction>,
            Option<ethrex_common::types::BlobsBundle>,
        )>();

    // Spawn forwarding task
    tokio::spawn(async move {
        while let Some((tx, blobs_bundle)) = tx_receiver.recv().await {
            if let Err(e) = handle
                .cast(ethrex_dev::CastMsg::SubmitTransaction { tx, blobs_bundle })
                .await
            {
                tracing::error!("Failed to forward transaction to block builder: {e}");
            }
        }
    });

    // Dummy SyncManager (dev mode doesn't sync)
    let dummy_store =
        Store::new("memory", EngineType::InMemory).map_err(|e| eyre::eyre!("{e}"))?;
    let dummy_blockchain = Arc::new(Blockchain::new(
        dummy_store.clone(),
        BlockchainOptions::default(),
    ));
    let cancel_token = CancellationToken::new();
    let peer_table = PeerTable::spawn(1);
    let dummy_peer_handler = {
        let p2p_context = ethrex_p2p::network::P2PContext::new(
            Node::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                0,
                0,
                ethrex_common::H512::zero(),
            ),
            TaskTracker::new(),
            SecretKey::new(&mut OsRng),
            peer_table.clone(),
            dummy_store.clone(),
            dummy_blockchain,
            String::new(),
            None,
            1000,
            100.0,
        )
        .map_err(|e| eyre::eyre!("P2P context error: {e}"))?;
        let initiator = RLPxInitiator::spawn(p2p_context).await;
        PeerHandler::new(peer_table.clone(), initiator)
    };

    let syncer = SyncManager::new(
        dummy_peer_handler.clone(),
        &SyncMode::Full,
        cancel_token.clone(),
        blockchain.clone(),
        store.clone(),
        ".".into(),
    )
    .await;

    let dummy_node = Node::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        0,
        0,
        ethrex_common::H512::zero(),
    );
    let dummy_signer = SecretKey::new(&mut OsRng);
    let local_node_record = NodeRecord::from_node(&dummy_node, 1, &dummy_signer)
        .map_err(|e| eyre::eyre!("Failed to create node record: {e}"))?;

    let ws_socket_opts = if opts.ws_enabled {
        Some(get_ws_socket_addr(&opts))
    } else {
        None
    };

    let tracker = TaskTracker::new();

    let rpc_api = ethrex_rpc::start_api(
        get_http_socket_addr(&opts),
        ws_socket_opts,
        None, // No auth RPC in dev mode
        store,
        blockchain,
        bytes::Bytes::new(),
        dummy_node,
        local_node_record.clone(),
        syncer,
        dummy_peer_handler,
        get_client_version(),
        log_filter_handler,
        opts.gas_limit,
        opts.extra_data.clone(),
        Some(tx_sender),
    );

    tracker.spawn(rpc_api);

    if opts.metrics_enabled {
        let network = get_network(&opts);
        init_metrics(&opts, &network, tracker);
    }

    info!(
        "Dev node listening on http://{}",
        get_http_socket_addr(&opts)
    );

    Ok((
        opts.datadir.clone(),
        cancel_token,
        peer_table,
        local_node_record,
    ))
}

pub fn get_network(opts: &Options) -> Network {
    let default = if opts.dev {
        Network::LocalDevnet
    } else {
        Network::mainnet()
    };
    opts.network.clone().unwrap_or(default)
}

pub fn get_bootnodes(opts: &Options, network: &Network, datadir: &Path) -> Vec<Node> {
    let mut bootnodes: Vec<Node> = opts.bootnodes.clone();

    bootnodes.extend(network.get_bootnodes());

    debug!("Loading known peers from config");

    match read_node_config_file(datadir) {
        Ok(Some(ref mut config)) => bootnodes.append(&mut config.known_peers),
        Ok(None) => {} // No config file, nothing to do
        Err(e) => warn!("Could not read from peers file: {e}"),
    };

    if bootnodes.is_empty() {
        warn!("No bootnodes specified. This node will not be able to connect to the network.");
    }

    bootnodes
}

pub fn get_signer(datadir: &Path) -> SecretKey {
    if is_memory_datadir(datadir) {
        return SecretKey::new(&mut OsRng);
    }

    // Get the signer from the default directory, create one if the key file is not present.
    let key_path = datadir.join("node.key");
    match fs::read(key_path.clone()) {
        Ok(content) => SecretKey::from_slice(&content).expect("Signing key could not be created."),
        Err(_) => {
            info!(
                "Key file not found, creating a new key and saving to {:?}",
                key_path
            );
            if let Some(parent) = key_path.parent() {
                fs::create_dir_all(parent).expect("Key file path could not be created.")
            }
            let signer = SecretKey::new(&mut OsRng);
            fs::write(key_path, signer.secret_bytes())
                .expect("Newly created signer could not be saved to disk.");
            signer
        }
    }
}

pub fn get_local_p2p_node(opts: &Options, signer: &SecretKey) -> Node {
    let tcp_port = opts.p2p_port.parse().expect("Failed to parse p2p port");
    let udp_port = opts
        .discovery_port
        .parse()
        .expect("Failed to parse discovery port");

    let p2p_node_ip: IpAddr = if let Some(addr) = &opts.p2p_addr {
        addr.parse().expect("Failed to parse p2p address")
    } else {
        local_ip()
            .unwrap_or_else(|_| local_ipv6().expect("Neither ipv4 nor ipv6 local address found"))
    };

    let local_public_key = public_key_from_signing_key(signer);

    let node = Node::new(p2p_node_ip, udp_port, tcp_port, local_public_key);

    // TODO Find a proper place to show node information
    // https://github.com/lambdaclass/ethrex/issues/836
    let enode = node.enode_url();
    info!(enode = %enode, "Local node initialized");

    node
}

pub fn get_local_node_record(
    datadir: &Path,
    local_p2p_node: &Node,
    signer: &SecretKey,
) -> NodeRecord {
    match read_node_config_file(datadir) {
        Ok(Some(ref mut config)) => {
            NodeRecord::from_node(local_p2p_node, config.node_record.seq + 1, signer)
                .expect("Node record could not be created from local node")
        }
        _ => {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            NodeRecord::from_node(local_p2p_node, timestamp, signer)
                .expect("Node record could not be created from local node")
        }
    }
}

pub fn get_authrpc_socket_addr(opts: &Options) -> SocketAddr {
    parse_socket_addr(&opts.authrpc_addr, &opts.authrpc_port)
        .expect("Failed to parse authrpc address and port")
}

pub fn get_http_socket_addr(opts: &Options) -> SocketAddr {
    parse_socket_addr(&opts.http_addr, &opts.http_port)
        .expect("Failed to parse http address and port")
}

pub fn get_ws_socket_addr(opts: &Options) -> SocketAddr {
    parse_socket_addr(&opts.ws_addr, &opts.ws_port)
        .expect("Failed to parse websocket address and port")
}

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
            .forkchoice_update(vec![], block_number, block_hash, None, None)
            .await
            .expect("Could not set sync block");
    }
}

pub async fn init_l1(
    opts: Options,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
) -> eyre::Result<(PathBuf, CancellationToken, PeerTable, NodeRecord)> {
    #[cfg(feature = "dev")]
    if opts.dev {
        return init_l1_dev(opts, log_filter_handler).await;
    }

    let datadir: &PathBuf = &opts.datadir;

    if !is_memory_datadir(datadir) {
        init_datadir(datadir);
    }

    let network = get_network(&opts);

    let genesis = network.get_genesis()?;
    display_chain_initialization(&genesis);

    raise_fd_limit()?;
    debug!("Preloading KZG trusted setup");
    ethrex_crypto::kzg::warm_up_trusted_setup();

    let store = match init_store(datadir, genesis).await {
        Ok(store) => store,
        Err(err @ StoreError::IncompatibleDBVersion { .. })
        | Err(err @ StoreError::NotFoundDBVersion { .. }) => {
            return Err(eyre::eyre!(
                "{err}. Please erase your DB by running `ethrex removedb` and restart node to resync. Note that this will take a while."
            ));
        }
        Err(error) => return Err(eyre::eyre!("Failed to create Store: {error}")),
    };

    if opts.syncmode == SyncMode::Full {
        store.generate_flatkeyvalue()?;
    }

    #[cfg(feature = "sync-test")]
    set_sync_block(&store).await;

    let blockchain = init_blockchain(
        store.clone(),
        BlockchainOptions {
            max_mempool_size: opts.mempool_max_size,
            perf_logs_enabled: true,
            r#type: BlockchainType::L1,
            max_blobs_per_block: opts.max_blobs_per_block,
            precompute_witnesses: opts.precompute_witnesses,
        },
    );

    regenerate_head_state(&store, &blockchain).await?;

    let signer = get_signer(datadir);

    let local_p2p_node = get_local_p2p_node(&opts, &signer);

    let local_node_record = get_local_node_record(datadir, &local_p2p_node, &signer);

    let peer_table = PeerTable::spawn(opts.target_peers);

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();

    let cancel_token = tokio_util::sync::CancellationToken::new();

    let p2p_context = P2PContext::new(
        local_p2p_node.clone(),
        tracker.clone(),
        signer,
        peer_table.clone(),
        store.clone(),
        blockchain.clone(),
        get_client_version(),
        None,
        opts.tx_broadcasting_time_interval,
        opts.lookup_interval,
    )
    .expect("P2P context could not be created");

    let initiator = RLPxInitiator::spawn(p2p_context.clone()).await;

    let peer_handler = PeerHandler::new(peer_table.clone(), initiator);

    init_rpc_api(
        &opts,
        peer_handler.clone(),
        local_p2p_node,
        local_node_record.clone(),
        store.clone(),
        blockchain.clone(),
        cancel_token.clone(),
        tracker.clone(),
        log_filter_handler,
    )
    .await;

    if opts.metrics_enabled {
        init_metrics(&opts, &network, tracker.clone());
    }

    if !opts.p2p_disabled {
        init_network(
            &opts,
            &network,
            datadir,
            peer_handler.clone(),
            tracker.clone(),
            blockchain.clone(),
            p2p_context,
        )
        .await;
    } else {
        info!("P2P is disabled");
    }

    Ok((
        datadir.clone(),
        cancel_token,
        peer_handler.peer_table,
        local_node_record,
    ))
}

/// Regenerates the state up to the head block by re-applying blocks from the
/// last known state root.
///
/// Since the path-based feature was added, the database stores the state 128
/// blocks behind the head block while the state of the blocks in between are
/// kept in in-memory-diff-layers.
///
/// After the node is shut down, those in-memory layers are lost, and the database
/// won't have the state for those blocks. It will have the blocks though.
///
/// When the node is started again, the state needs to be regenerated by
/// re-applying the blocks from the last known state root up to the head block.
///
/// This function performs that regeneration.
pub async fn regenerate_head_state(
    store: &Store,
    blockchain: &Arc<Blockchain>,
) -> eyre::Result<()> {
    let head_block_number = store.get_latest_block_number().await?;

    let Some(last_header) = store.get_block_header(head_block_number)? else {
        unreachable!("Database is empty, genesis block should be present");
    };

    let mut current_last_header = last_header;

    // Find the last block with a known state root
    while !store.has_state_root(current_last_header.state_root)? {
        if current_last_header.number == 0 {
            return Err(eyre::eyre!(
                "Unknown state found in DB. Please run `ethrex removedb` and restart node"
            ));
        }
        let parent_number = current_last_header.number - 1;

        debug!("Need to regenerate state for block {parent_number}");

        let Some(parent_header) = store.get_block_header(parent_number)? else {
            return Err(eyre::eyre!(
                "Parent header for block {parent_number} not found"
            ));
        };

        current_last_header = parent_header;
    }

    let last_state_number = current_last_header.number;

    if last_state_number == head_block_number {
        debug!("State is already up to date");
        return Ok(());
    }

    info!("Regenerating state from block {last_state_number} to {head_block_number}");

    // Re-apply blocks from the last known state root to the head block
    for i in (last_state_number + 1)..=head_block_number {
        debug!("Re-applying block {i} to regenerate state");

        let block = store
            .get_block_by_number(i)
            .await?
            .ok_or_else(|| eyre::eyre!("Block {i} not found"))?;

        blockchain.add_block_pipeline(block)?;
    }

    info!("Finished regenerating state");

    Ok(())
}
