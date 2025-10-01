use crate::{
    cli::Options,
    utils::{
        display_chain_initialization, get_client_version, init_datadir, parse_socket_addr,
        read_jwtsecret_file, read_node_config_file,
    },
};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType};
use ethrex_common::types::Genesis;
use ethrex_config::networks::Network;

use ethrex_metrics::profiling::{FunctionProfilingLayer, initialize_block_processing_profile};
use ethrex_p2p::{
    discv4::peer_table::{PeerTable, PeerTableHandle},
    network::P2PContext,
    peer_handler::PeerHandler,
    rlpx::l2::l2_connection::P2PBasedContext,
    sync_manager::SyncManager,
    types::{Node, NodeRecord},
    utils::public_key_from_signing_key,
};
use ethrex_storage::{EngineType, Store};
use local_ip_address::{local_ip, local_ipv6};
use rand::rngs::OsRng;
use secp256k1::SecretKey;
#[cfg(feature = "sync-test")]
use std::env;
use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, filter::Directive, fmt, layer::SubscriberExt, reload,
};

pub fn init_tracing(opts: &Options) -> reload::Handle<EnvFilter, Registry> {
    let log_filter = EnvFilter::builder()
        .with_default_directive(Directive::from(opts.log_level))
        .from_env_lossy();

    let (filter, filter_handle) = reload::Layer::new(log_filter);

    let fmt_layer = fmt::layer().with_filter(filter);
    let subscriber: Box<dyn tracing::Subscriber + Send + Sync> = if opts.metrics_enabled {
        let profiling_layer = FunctionProfilingLayer::default();
        Box::new(Registry::default().with(fmt_layer).with(profiling_layer))
    } else {
        Box::new(Registry::default().with(fmt_layer))
    };

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    filter_handle
}

pub fn init_metrics(opts: &Options, tracker: TaskTracker) {
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

    tracker.spawn(metrics_api);
}

/// Opens a new or pre-existing Store and loads the initial state provided by the network
pub async fn init_store(datadir: impl AsRef<Path>, genesis: Genesis) -> Store {
    let store = open_store(datadir.as_ref());
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to create genesis block");
    store
}

/// Initializes a pre-existing Store
pub async fn load_store(datadir: &Path) -> Store {
    let store = open_store(datadir);
    store
        .load_initial_state()
        .await
        .expect("Failed to load store");
    store
}

/// Opens a pre-existing Store or creates a new one
pub fn open_store(datadir: &Path) -> Store {
    if datadir.ends_with("memory") {
        Store::new(datadir, EngineType::InMemory).expect("Failed to create Store")
    } else {
        cfg_if::cfg_if! {
            if #[cfg(feature = "rocksdb")] {
                let engine_type = EngineType::RocksDB;
            } else if #[cfg(feature = "libmdbx")] {
                let engine_type = EngineType::Libmdbx;
            } else {
                error!("No database specified. The feature flag `rocksdb` or `libmdbx` should've been set while building.");
                panic!("Specify the desired database engine.");
            }
        };
        Store::new(datadir, engine_type).expect("Failed to create Store")
    }
}

pub fn init_blockchain(store: Store, blockchain_opts: BlockchainOptions) -> Arc<Blockchain> {
    info!("Initiating blockchain with levm");
    Blockchain::new(store, blockchain_opts).into()
}

#[allow(clippy::too_many_arguments)]
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
    gas_ceil: Option<u64>,
    extra_data: String,
) {
    init_datadir(&opts.datadir);
    // Create SyncManager
    let syncer = SyncManager::new(
        peer_handler.clone(),
        opts.syncmode.clone(),
        cancel_token,
        blockchain.clone(),
        store.clone(),
        opts.datadir.clone(),
    )
    .await;

    let rpc_api = ethrex_rpc::start_api(
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
        log_filter_handler,
        gas_ceil,
        extra_data,
    );

    tracker.spawn(rpc_api);
}

#[allow(clippy::too_many_arguments)]
pub async fn init_network(
    opts: &Options,
    network: &Network,
    datadir: &Path,
    local_p2p_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SecretKey,
    peer_handler: PeerHandler,
    store: Store,
    tracker: TaskTracker,
    blockchain: Arc<Blockchain>,
    based_context: Option<P2PBasedContext>,
) {
    if opts.dev {
        error!("Binary wasn't built with The feature flag `dev` enabled.");
        panic!(
            "Build the binary with the `dev` feature in order to use the `--dev` cli's argument."
        );
    }

    let bootnodes = get_bootnodes(opts, network, datadir);

    let context = P2PContext::new(
        local_p2p_node,
        local_node_record,
        tracker.clone(),
        signer,
        peer_handler.peer_table.clone(),
        store,
        blockchain.clone(),
        get_client_version(),
        based_context,
    )
    .await
    .expect("P2P context could not be created");

    ethrex_p2p::start_network(context, bootnodes)
        .await
        .expect("Network starts");

    tracker.spawn(ethrex_p2p::periodically_show_peer_stats(
        blockchain,
        peer_handler.peer_table,
    ));
}

#[cfg(feature = "dev")]
pub async fn init_dev_network(opts: &Options, store: &Store, tracker: TaskTracker) {
    info!("Running in DEV_MODE");

    let head_block_hash = {
        let current_block_number = store.get_latest_block_number().await.unwrap();
        store
            .get_canonical_block_hash(current_block_number)
            .await
            .unwrap()
            .unwrap()
    };

    let max_tries = 3;

    let url = format!(
        "http://{authrpc_socket_addr}",
        authrpc_socket_addr = get_authrpc_socket_addr(opts)
    );

    let block_producer_engine = ethrex_dev::block_producer::start_block_producer(
        url,
        read_jwtsecret_file(&opts.authrpc_jwtsecret),
        head_block_hash,
        max_tries,
        1000,
        ethrex_common::Address::default(),
    );
    tracker.spawn(block_producer_engine);
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
    let udp_socket_addr = parse_socket_addr("::", &opts.discovery_port)
        .expect("Failed to parse discovery address and port");
    let tcp_socket_addr =
        parse_socket_addr("::", &opts.p2p_port).expect("Failed to parse addr and port");

    let p2p_node_ip = local_ip()
        .unwrap_or_else(|_| local_ipv6().expect("Neither ipv4 nor ipv6 local address found"));

    let local_public_key = public_key_from_signing_key(signer);

    let node = Node::new(
        p2p_node_ip,
        udp_socket_addr.port(),
        tcp_socket_addr.port(),
        local_public_key,
    );

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
            .forkchoice_update(None, block_number, block_hash, None, None)
            .await
            .expect("Could not set sync block");
    }
}

pub async fn init_l1(
    opts: Options,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
) -> eyre::Result<(
    PathBuf,
    CancellationToken,
    PeerTableHandle,
    Arc<Mutex<NodeRecord>>,
)> {
    let datadir = &opts.datadir;
    init_datadir(datadir);

    let network = get_network(&opts);

    let genesis = network.get_genesis()?;
    display_chain_initialization(&genesis);
    let store = init_store(datadir, genesis).await;

    #[cfg(feature = "sync-test")]
    set_sync_block(&store).await;

    let blockchain = init_blockchain(
        store.clone(),
        BlockchainOptions {
            max_mempool_size: opts.mempool_max_size,
            perf_logs_enabled: true,
            r#type: BlockchainType::L1,
        },
    );

    let signer = get_signer(datadir);

    let local_p2p_node = get_local_p2p_node(&opts, &signer);

    let local_node_record = Arc::new(Mutex::new(get_local_node_record(
        datadir,
        &local_p2p_node,
        &signer,
    )));

    let peer_handler = PeerHandler::new(PeerTable::spawn());

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();

    let cancel_token = tokio_util::sync::CancellationToken::new();

    init_rpc_api(
        &opts,
        peer_handler.clone(),
        local_p2p_node.clone(),
        local_node_record.lock().await.clone(),
        store.clone(),
        blockchain.clone(),
        cancel_token.clone(),
        tracker.clone(),
        log_filter_handler,
        // TODO (#4482): Make this configurable.
        None,
        opts.extra_data.clone(),
    )
    .await;

    if opts.metrics_enabled {
        init_metrics(&opts, tracker.clone());
    }

    if opts.dev {
        #[cfg(feature = "dev")]
        init_dev_network(&opts, &store, tracker.clone()).await;
    } else if opts.p2p_enabled {
        init_network(
            &opts,
            &network,
            datadir,
            local_p2p_node,
            local_node_record.clone(),
            signer,
            peer_handler.clone(),
            store.clone(),
            tracker.clone(),
            blockchain.clone(),
            None,
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
