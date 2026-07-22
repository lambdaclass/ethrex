use crate::{
    cli::{LogColor, Options},
    utils::{
        display_chain_initialization, get_channel, get_client_version, get_client_version_string,
        init_datadir, is_memory_datadir, parse_socket_addr, read_jwtsecret_file,
        read_node_config_file,
    },
};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType};
use ethrex_common::fd_limit::raise_fd_limit;
use ethrex_common::types::Genesis;
use ethrex_config::networks::Network;
use ethrex_rpc::WebSocketConfig;

use ethrex_metrics::profiling::{FunctionProfilingLayer, initialize_block_processing_profile};
use ethrex_metrics::rpc::initialize_rpc_metrics;
use ethrex_p2p::rlpx::initiator::RLPxInitiator;
use ethrex_p2p::{
    DiscoveryConfig,
    network::P2PContext,
    peer_handler::PeerHandler,
    peer_table::{PeerTable, PeerTableServer},
    sync::{BackfillConfig, HistoryChain, SyncMode},
    sync_manager::SyncManager,
    types::{NetworkConfig, Node, NodeRecord},
    utils::public_key_from_signing_key,
};
use ethrex_storage::{
    EngineType, Store, StoreConfig, error::StoreError, has_valid_db, read_chain_id_from_db,
};
use local_ip_address::{local_ip, local_ipv6};
use rand::rngs::OsRng;
use secp256k1::SecretKey;
#[cfg(feature = "sync-test")]
use std::env;
use std::{
    fs,
    io::IsTerminal,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{Level, debug, error, info, warn};
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

        let branch = get_channel().replace('/', "-");
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
        &get_channel(),
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

    // Metrics is a non-fatal sidecar: its failure is logged loudly but must not down the node.
    spawn_logged(&tracker, "metrics server", metrics_api);
}

/// Opens a new or pre-existing Store with default tunables and loads the initial
/// state provided by the network. See [`init_store_with_config`] for the variant
/// that lets production callers thread CLI-provided storage tunables through.
pub async fn init_store(datadir: impl AsRef<Path>, genesis: Genesis) -> Result<Store, StoreError> {
    init_store_with_config(datadir, genesis, StoreConfig::default()).await
}

/// Opens a Store with the supplied [`StoreConfig`] and loads the initial state.
pub async fn init_store_with_config(
    datadir: impl AsRef<Path>,
    genesis: Genesis,
    config: StoreConfig,
) -> Result<Store, StoreError> {
    let mut store = open_store_with_config(datadir.as_ref(), config)?;
    store.add_initial_state(genesis).await?;
    Ok(store)
}

/// Like [`init_store`], but trusts a pre-existing datadir's genesis instead of
/// validating it against `genesis`. See [`Store::add_initial_state_skip_validation`].
pub async fn init_store_skip_validation(
    datadir: impl AsRef<Path>,
    genesis: Genesis,
) -> Result<Store, StoreError> {
    init_store_skip_validation_with_config(datadir, genesis, StoreConfig::default()).await
}

/// Like [`init_store_with_config`], but trusts a pre-existing datadir's genesis
/// instead of validating it against `genesis`.
pub async fn init_store_skip_validation_with_config(
    datadir: impl AsRef<Path>,
    genesis: Genesis,
    config: StoreConfig,
) -> Result<Store, StoreError> {
    let mut store = open_store_with_config(datadir.as_ref(), config)?;
    store.add_initial_state_skip_validation(genesis).await?;
    Ok(store)
}

/// Initializes a pre-existing Store with default tunables. See [`load_store_with_config`].
pub async fn load_store(datadir: &Path) -> Result<Store, StoreError> {
    load_store_with_config(datadir, StoreConfig::default()).await
}

/// Initializes a pre-existing Store, applying the supplied [`StoreConfig`].
pub async fn load_store_with_config(
    datadir: &Path,
    config: StoreConfig,
) -> Result<Store, StoreError> {
    let store = open_store_with_config(datadir, config)?;
    store.load_initial_state().await?;
    Ok(store)
}

/// Opens a pre-existing Store or creates a new one with default tunables.
/// See [`open_store_with_config`].
pub fn open_store(datadir: &Path) -> Result<Store, StoreError> {
    open_store_with_config(datadir, StoreConfig::default())
}

/// Opens a pre-existing Store or creates a new one, applying the supplied [`StoreConfig`].
pub fn open_store_with_config(datadir: &Path, config: StoreConfig) -> Result<Store, StoreError> {
    if is_memory_datadir(datadir) {
        Store::new_with_config(datadir, EngineType::InMemory, config)
    } else {
        #[cfg(feature = "rocksdb")]
        let engine_type = EngineType::RocksDB;
        #[cfg(feature = "metrics")]
        ethrex_metrics::process::set_datadir_path(datadir.to_path_buf());
        Store::new_with_config(datadir, engine_type, config)
    }
}

pub fn init_blockchain(store: Store, blockchain_opts: BlockchainOptions) -> Arc<Blockchain> {
    info!("Initiating blockchain with levm");
    Blockchain::new(store, blockchain_opts).into()
}

/// Cause of a fatal-subsystem shutdown, set by [`spawn_fatal`] before it cancels the node.
/// `main` inspects it after the shutdown sequence to exit non-zero on a fatal-initiated
/// shutdown (signal-triggered shutdowns leave it unset and exit zero).
static FATAL_SHUTDOWN_CAUSE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Returns the fatal-subsystem failure that initiated shutdown, if any.
pub fn fatal_shutdown_cause() -> Option<&'static str> {
    FATAL_SHUTDOWN_CAUSE.get().map(String::as_str)
}

/// Spawns a subsystem whose failure is fatal to the node. On an error raised *before*
/// shutdown has begun it logs loudly, records the cause (so `main` exits non-zero), and
/// cancels the node's token so the main loop tears everything down. An error surfacing
/// *after* cancellation (e.g. a client dropping during a graceful drain) is downgraded to
/// a debug line and does not re-cancel — this keeps the operator-facing shutdown reason
/// honest.
pub(crate) fn spawn_fatal<F, E>(
    tracker: &TaskTracker,
    cancel_token: CancellationToken,
    name: &'static str,
    fut: F,
) where
    F: std::future::Future<Output = Result<(), E>> + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    tracker.spawn(async move {
        match fut.await {
            Ok(()) => {}
            Err(err) if cancel_token.is_cancelled() => {
                debug!("{name} returned after shutdown began: {err}");
            }
            Err(err) => {
                error!("{name} failed: {err}; shutting down the node");
                let _ = FATAL_SHUTDOWN_CAUSE.set(format!("{name}: {err}"));
                cancel_token.cancel();
            }
        }
    });
}

/// Spawns a non-fatal subsystem: an error is logged loudly but the node keeps running.
pub(crate) fn spawn_logged<F, E>(tracker: &TaskTracker, name: &'static str, fut: F)
where
    F: std::future::Future<Output = Result<(), E>> + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    tracker.spawn(async move {
        if let Err(err) = fut.await {
            error!("{name} exited with error: {err}");
        }
    });
}

#[expect(clippy::too_many_arguments)]
pub async fn init_rpc_api(
    opts: &Options,
    datadir: &Path,
    peer_handler: PeerHandler,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    store: Store,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    tracker: TaskTracker,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
) -> eyre::Result<()> {
    if !is_memory_datadir(datadir) {
        init_datadir(datadir);
    }

    let syncmode = if opts.dev {
        &SyncMode::Full
    } else {
        &opts.syncmode
    };

    // Historical-chain backfill is opt-in via `--history.chain`; it is
    // meaningless in dev mode (single-node chain, full state from genesis), so
    // force it off there like syncmode.
    let backfill_config = BackfillConfig {
        mode: if opts.dev {
            HistoryChain::Off
        } else {
            opts.history_chain.clone()
        },
        tx_index_horizon: opts.history_transactions,
    };

    // Create SyncManager
    let syncer = SyncManager::new(
        peer_handler.clone(),
        syncmode,
        cancel_token.clone(),
        blockchain.clone(),
        store.clone(),
        datadir.to_path_buf(),
        backfill_config,
    )
    .await;

    let ws_config = if opts.ws_enabled {
        Some(WebSocketConfig {
            addr: get_ws_socket_addr(opts),
            subscription_manager: ethrex_rpc::SubscriptionManager::spawn(),
        })
    } else {
        None
    };

    // Reject conflicting listener addresses at config time, before anything binds, with an
    // error naming both flags to change.
    validate_rpc_addrs(
        get_http_socket_addr(opts),
        Some(get_authrpc_socket_addr(opts)),
        ws_config.as_ref().map(|ws| ws.addr),
    )?;

    // Bind in the foreground so a failure (e.g. a port collision) aborts node startup with
    // an actionable error, instead of being swallowed by a detached task. Serving runs in
    // the background once every listener is bound.
    let bound = ethrex_rpc::bind_api(
        cancel_token.clone(),
        get_http_socket_addr(opts),
        ws_config,
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
        opts.gas_limit,
        opts.extra_data.clone(),
        opts.http_api.iter().copied().collect(),
    )
    .await?;

    // Defensive wiring: axum's serve loop retries accept errors internally and only returns
    // after graceful shutdown, so today this error arm is unreachable for the RPC server. It
    // exists so any future serve error (an axum behavior change, a refactor) aborts the node
    // instead of being silently dropped — a node without its Engine API cannot sync.
    spawn_fatal(&tracker, cancel_token, "RPC server", bound.serve());
    Ok(())
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

    let discovery_config = DiscoveryConfig {
        discv4_enabled: opts.discv4_enabled,
        discv5_enabled: opts.discv5_enabled,
        ..Default::default()
    };

    ethrex_p2p::start_network(context, bootnodes, discovery_config)
        .await
        .expect("Network starts");

    tracker.spawn(ethrex_p2p::periodically_show_peer_stats(
        blockchain,
        peer_handler.peer_table,
    ));
}

#[cfg(feature = "dev")]
pub async fn init_dev_network(
    opts: &Options,
    store: &Store,
    tracker: TaskTracker,
    cancel_token: CancellationToken,
) {
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
    // The dev block producer is fatal: if it exhausts its retries, abort the dev node.
    spawn_fatal(
        &tracker,
        cancel_token,
        "block producer",
        block_producer_engine,
    );
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

/// Decide the bind and externally-announced addresses for the P2P endpoint.
///
/// Precedence:
/// - `--nat.extip` wins for the announced address; bind comes from `--p2p.addr` if given,
///   else the unspecified address of the matching family.
/// - `--p2p.addr` alone is used for both bind and announce, except when it's an unspecified
///   address (`0.0.0.0` / `::`). In that case the announced address falls back to the
///   auto-detected local IP of the matching family; this avoids advertising `0.0.0.0` in
///   the ENR, which would make the node unreachable for inbound connections. Operators
///   behind NAT still need `--nat.extip` for that case to resolve correctly.
/// - With neither flag set, the auto-detected local IP is used for both bind and announce.
fn resolve_p2p_endpoints(
    p2p_addr: Option<&str>,
    nat_extip: Option<&str>,
    local_v4: Option<IpAddr>,
    local_v6: Option<IpAddr>,
) -> (IpAddr, IpAddr) {
    match (p2p_addr, nat_extip) {
        (_, Some(extip)) => {
            let external: IpAddr = extip.parse().expect("Failed to parse --nat.extip address");
            let bind: IpAddr = p2p_addr
                .map(|a| {
                    let addr: IpAddr = a.parse().expect("Failed to parse p2p address");
                    assert!(
                        addr.is_ipv4() == external.is_ipv4(),
                        "--p2p.addr and --nat.extip must use the same address family (both IPv4 or both IPv6)"
                    );
                    addr
                })
                .unwrap_or_else(|| {
                    if external.is_ipv6() {
                        IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
                    } else {
                        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                    }
                });
            (bind, external)
        }
        (Some(addr), None) => {
            let bind: IpAddr = addr.parse().expect("Failed to parse p2p address");
            if bind.is_unspecified() {
                // Stay in the same address family: an IPv4 socket can't accept
                // inbound IPv6 connections (and vice versa), so falling back
                // across families would just advertise an unreachable address.
                let external = if bind.is_ipv6() { local_v6 } else { local_v4 };
                match external {
                    Some(ext) => {
                        info!(
                            announced = %ext,
                            bind = %bind,
                            "--p2p.addr is unspecified; announcing auto-detected local IP. Set --nat.extip to override."
                        );
                        (bind, ext)
                    }
                    None => {
                        warn!(
                            bind = %bind,
                            "--p2p.addr is unspecified and no local IP could be detected; \
                             announcing the unspecified address. Inbound peer connections will fail. \
                             Set --nat.extip=<ip> or --p2p.addr=<ip> to fix."
                        );
                        (bind, bind)
                    }
                }
            } else {
                (bind, bind)
            }
        }
        (None, None) => {
            let ip = local_v4
                .or(local_v6)
                .expect("Neither ipv4 nor ipv6 local address found");
            (ip, ip)
        }
    }
}

pub fn get_local_p2p_node(opts: &Options, signer: &SecretKey) -> (Node, NetworkConfig) {
    let tcp_port = opts.p2p_port.parse().expect("Failed to parse p2p port");
    let udp_port = opts
        .discovery_port
        .parse()
        .expect("Failed to parse discovery port");

    let local_public_key = public_key_from_signing_key(signer);

    let (bind_addr, external_addr) = resolve_p2p_endpoints(
        opts.p2p_addr.as_deref(),
        opts.nat_extip.as_deref(),
        local_ip().ok(),
        local_ipv6().ok(),
    );

    let node = Node::new(external_addr, udp_port, tcp_port, local_public_key);
    let network_config = NetworkConfig {
        bind_addr,
        tcp_port,
        udp_port,
    };

    // TODO Find a proper place to show node information
    // https://github.com/lambdaclass/ethrex/issues/836
    let enode = node.enode_url();
    info!(enode = %enode, "Local node initialized");

    (node, network_config)
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

/// Two configured listener addresses conflict when they are equal, or when they share a
/// port and one side is a same-family wildcard: Linux fails the second bind with
/// EADDRINUSE, but on macOS/BSD `SO_REUSEADDR` lets the specific bind succeed and silently
/// shadow the wildcard listener for that address.
fn rpc_addrs_conflict(a: SocketAddr, b: SocketAddr) -> bool {
    a == b
        || (a.port() == b.port()
            && a.is_ipv4() == b.is_ipv4()
            && (a.ip().is_unspecified() || b.ip().is_unspecified()))
}

/// Validates the resolved RPC listener addresses at config time, before anything binds, so
/// a conflict aborts startup with an error naming BOTH flags to change (an OS bind error
/// can only ever blame the second binder). A WebSocket address exactly equal to the HTTP
/// one is not a conflict: both protocols share that listener.
pub(crate) fn validate_rpc_addrs(
    http: SocketAddr,
    authrpc: Option<SocketAddr>,
    ws: Option<SocketAddr>,
) -> eyre::Result<()> {
    use ethrex_rpc::RpcRole;

    // WS equal to HTTP shares the HTTP listener instead of binding its own.
    let ws = ws.filter(|ws| *ws != http);

    let http = (RpcRole::Http, http);
    let authrpc = authrpc.map(|addr| (RpcRole::AuthRpc, addr));
    let ws = ws.map(|addr| (RpcRole::Ws, addr));
    let pairs = [
        authrpc.map(|authrpc| (http, authrpc)),
        ws.map(|ws| (http, ws)),
        authrpc.zip(ws),
    ];
    for ((role_a, a), (role_b, b)) in pairs.into_iter().flatten() {
        if !rpc_addrs_conflict(a, b) {
            continue;
        }
        if a == b {
            eyre::bail!(
                "{a} is requested by both the {role_a} and the {role_b}; change {} or {}.",
                role_a.flags(),
                role_b.flags(),
            );
        }
        eyre::bail!(
            "{b} ({role_b}) overlaps {a} ({role_a}): a wildcard address covers every \
             interface on its port; change {} or {}.",
            role_a.flags(),
            role_b.flags(),
        );
    }
    Ok(())
}

pub fn get_ws_socket_addr(opts: &Options) -> SocketAddr {
    // When unset, WebSocket inherits the HTTP address/port, so an enabled WS shares the
    // HTTP listener by default (a single-port setup, matching geth/reth/nethermind).
    let addr = opts.ws_addr.as_deref().unwrap_or(&opts.http_addr);
    let port = opts.ws_port.as_deref().unwrap_or(&opts.http_port);
    let resolved =
        parse_socket_addr(addr, port).expect("Failed to parse websocket address and port");
    // Warn on the RESOLVED address (explicit or inherited) so L1 and L2 alike surface a
    // publicly reachable WebSocket bind.
    if !resolved.ip().is_loopback() {
        warn!(
            "WebSocket RPC is bound to {resolved}, reachable from all matching interfaces; bind 127.0.0.1 (the default) unless it sits behind a trusted proxy."
        );
    }
    resolved
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
) -> eyre::Result<(PathBuf, CancellationToken, PeerTable, NodeRecord, Store)> {
    let network = get_network(&opts);
    let datadir = crate::cli::compute_effective_datadir(&opts.datadir, &network, opts.dev);

    raise_fd_limit()?;

    migrate_datadir_if_needed(&opts.datadir, &datadir, &network, opts.no_migrate);

    if !is_memory_datadir(&datadir) {
        init_datadir(&datadir);
    }

    let genesis = network.get_genesis()?;
    display_chain_initialization(&genesis);
    debug!("Preloading KZG trusted setup");
    ethrex_crypto::kzg::warm_up_trusted_setup();

    let store_config = StoreConfig {
        rocksdb_block_cache_size: opts.rocksdb_block_cache_size,
        ..StoreConfig::default()
    };
    let store_result = if opts.skip_genesis_validation {
        init_store_skip_validation_with_config(&datadir, genesis, store_config).await
    } else {
        init_store_with_config(&datadir, genesis, store_config).await
    };
    let store = match store_result {
        Ok(store) => store,
        Err(err @ StoreError::IncompatibleDBVersion { .. })
        | Err(err @ StoreError::NotFoundDBVersion) => {
            return Err(eyre::eyre!(
                "{err}. Please erase your DB by running `ethrex removedb` and restart node to resync. Note that this will take a while."
            ));
        }
        Err(err @ StoreError::MigrationFailed { .. }) => {
            return Err(eyre::eyre!(
                "{err}. The database may be in an inconsistent state. Please erase your DB by running `ethrex removedb` and restart node to resync."
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
            precompile_cache_enabled: !opts.no_precompile_cache,
            max_queued_txs_per_account: opts.mempool_max_queued_txs_per_account,
            bal_parallel_exec_enabled: !opts.no_bal_parallel_exec,
            bal_prefetch_enabled: !opts.no_bal_prefetch,
            bal_parallel_trie_enabled: !opts.no_bal_parallel_trie,
            gap_admit_occupancy_threshold: opts.mempool_gap_admit_occupancy_threshold,
        },
    );

    regenerate_head_state(&store, &blockchain).await?;

    let signer = get_signer(&datadir);

    let (local_p2p_node, network_config) = get_local_p2p_node(&opts, &signer);

    let local_node_record = get_local_node_record(&datadir, &local_p2p_node, &signer);

    let peer_table =
        PeerTableServer::spawn(local_p2p_node.node_id(), opts.target_peers, store.clone());

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();

    let cancel_token = tokio_util::sync::CancellationToken::new();

    let p2p_context = P2PContext::new(
        local_p2p_node.clone(),
        network_config,
        tracker.clone(),
        signer,
        peer_table.clone(),
        store.clone(),
        blockchain.clone(),
        get_client_version_string(),
        None,
        opts.tx_broadcasting_time_interval,
        opts.lookup_interval,
    )
    .expect("P2P context could not be created");

    let initiator = RLPxInitiator::spawn(p2p_context.clone());

    let peer_handler = PeerHandler::new(peer_table.clone(), initiator);

    init_rpc_api(
        &opts,
        &datadir,
        peer_handler.clone(),
        local_p2p_node,
        local_node_record.clone(),
        store.clone(),
        blockchain.clone(),
        cancel_token.clone(),
        tracker.clone(),
        log_filter_handler,
    )
    .await?;

    if opts.metrics_enabled {
        init_metrics(&opts, &network, tracker.clone());
    }

    if opts.dev {
        #[cfg(feature = "dev")]
        init_dev_network(&opts, &store, tracker.clone(), cancel_token.clone()).await;
    } else if !opts.p2p_disabled {
        init_network(
            &opts,
            &network,
            &datadir,
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
        store,
    ))
}

/// Migrates data from a pre-suffix datadir layout to the new network-specific
/// subdirectory. Migration happens automatically unless `--no-migrate` is set.
///
/// Migration is performed when ALL of the following hold:
/// - `base_datadir != network_datadir` (a suffix was applied)
/// - The network-specific dir does not already contain a valid DB
/// - The base dir contains a valid DB with a matching chain ID
/// - No other network subdirectories exist in the base dir
/// - `no_migrate` is `false`
pub fn migrate_datadir_if_needed(
    base_datadir: &Path,
    network_datadir: &Path,
    network: &Network,
    no_migrate: bool,
) {
    // No suffix applied — nothing to migrate.
    if base_datadir == network_datadir {
        return;
    }

    // Network dir already has data — nothing to do.
    if has_valid_db(network_datadir) {
        return;
    }

    // Base dir has no DB — nothing to migrate from.
    if !has_valid_db(base_datadir) {
        return;
    }

    // Check that no network subdirectories already exist (avoids partial migration).
    for suffix in Network::all_datadir_suffixes() {
        let subdir = base_datadir.join(suffix);
        if subdir.exists() && subdir.is_dir() {
            info!("Found existing network subdirectory {subdir:?}, skipping migration.");
            return;
        }
    }

    // Verify chain IDs match.
    let Some(db_chain_id) = read_chain_id_from_db(base_datadir) else {
        warn!(
            "Found a database at {base_datadir:?} with valid store metadata but could not \
             read its chain ID. Skipping automatic migration to {network_datadir:?}. \
             If this is a pre-v10 database you intend to reuse, stop ethrex and move its \
             contents into {network_datadir:?} manually before restarting. See the logs \
             above for the specific error from the storage layer."
        );
        return;
    };
    let expected_chain_id = match network.get_genesis() {
        Ok(genesis) => genesis.config.chain_id,
        Err(_) => return,
    };
    if db_chain_id != expected_chain_id {
        warn!(
            "Existing database at {base_datadir:?} has chain ID {db_chain_id}, \
             expected {expected_chain_id} for {network}. Skipping migration."
        );
        return;
    }

    if no_migrate {
        info!(
            "Existing database at {base_datadir:?} can be migrated to {network_datadir:?}. \
             Skipping because --no-migrate is set."
        );
        return;
    }

    // All checks passed — migrate automatically.
    info!("Migrating existing database from {base_datadir:?} to {network_datadir:?}.");
    {
        if let Err(e) = std::fs::create_dir_all(network_datadir) {
            warn!("Failed to create {network_datadir:?}: {e}");
            return;
        }
        // Collect entries to move.
        let entries: Vec<_> = match std::fs::read_dir(base_datadir) {
            Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
            Err(e) => {
                warn!("Failed to read {base_datadir:?}: {e}");
                return;
            }
        };
        let network_dir_name = network_datadir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Build the list of (src, dest) pairs, skipping the network subdir itself.
        let moves: Vec<_> = entries
            .iter()
            .filter(|entry| entry.file_name().to_string_lossy() != network_dir_name)
            .map(|entry| (entry.path(), network_datadir.join(entry.file_name())))
            .collect();

        // Dry-run: verify no destination already exists.
        for (src, dest) in &moves {
            if dest.exists() {
                warn!(
                    "Destination {dest:?} already exists, aborting migration. \
                     Source {src:?} is untouched."
                );
                return;
            }
        }

        // Perform the actual moves.
        for (src, dest) in &moves {
            if let Err(e) = std::fs::rename(src, dest) {
                // Attempt to rollback already-moved files.
                warn!("Failed to move {src:?} to {dest:?}: {e}. Rolling back.");
                for (orig_src, orig_dest) in &moves {
                    if orig_dest.exists()
                        && !orig_src.exists()
                        && let Err(re) = std::fs::rename(orig_dest, orig_src)
                    {
                        warn!("Rollback failed for {orig_dest:?} -> {orig_src:?}: {re}");
                    }
                }
                warn!("Migration aborted. Database remains at {base_datadir:?}.");
                return;
            }
        }
        info!("Database migrated to {network_datadir:?}.");
    }
}

/// Re-apply blocks from the last on-disk state root up to the head block,
/// rebuilding the in-memory trie diff-layers lost across a restart.
pub async fn regenerate_head_state(
    store: &Store,
    blockchain: &Arc<Blockchain>,
) -> eyre::Result<()> {
    // Precondition: the store was opened via `add_initial_state`/`load_initial_state`,
    // which clamp `LatestBlockNumber` to `flushed_upto`. All blocks up to
    // `head_block_number` are therefore on disk; callers that skip that clamp
    // would break this assumption.
    let head_block_number = store.get_latest_block_number().await?;
    debug!("regenerate_head_state head clamped to durable block {head_block_number}");

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

        blockchain.add_block_pipeline(block, None)?;
    }

    info!("Finished regenerating state");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{resolve_p2p_endpoints, validate_rpc_addrs};
    use std::net::{IpAddr, SocketAddr};

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    /// The default layout (distinct ports) must validate.
    #[test]
    fn distinct_rpc_addrs_are_valid() {
        let result = validate_rpc_addrs(
            addr("127.0.0.1:8545"),
            Some(addr("127.0.0.1:8551")),
            Some(addr("127.0.0.1:8546")),
        );
        assert!(result.is_ok());
    }

    /// WebSocket exactly equal to HTTP shares the HTTP listener (merged single-port
    /// setup) — it must NOT be reported as a conflict.
    #[test]
    fn ws_sharing_the_http_listener_is_not_a_conflict() {
        let result = validate_rpc_addrs(
            addr("127.0.0.1:8545"),
            Some(addr("127.0.0.1:8551")),
            Some(addr("127.0.0.1:8545")),
        );
        assert!(result.is_ok());
    }

    /// A duplicate address must fail at config time with an error naming BOTH flags,
    /// since an OS bind error can only ever blame the second binder.
    #[test]
    fn duplicate_rpc_addr_names_both_flags() {
        let err = validate_rpc_addrs(
            addr("127.0.0.1:8545"),
            Some(addr("127.0.0.1:8551")),
            Some(addr("127.0.0.1:8551")),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Auth-RPC server"), "{err}");
        assert!(err.contains("WebSocket server"), "{err}");
        assert!(err.contains("--authrpc.port"), "{err}");
        assert!(err.contains("--ws.port"), "{err}");
    }

    /// A same-family wildcard on the same port covers the specific address: Linux fails
    /// the second bind, macOS/BSD lets it shadow the wildcard. Both must be rejected up
    /// front, uniformly.
    #[test]
    fn wildcard_overlap_is_rejected() {
        let err = validate_rpc_addrs(
            addr("0.0.0.0:8545"),
            Some(addr("127.0.0.1:8551")),
            Some(addr("127.0.0.1:8545")),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("overlaps"), "{err}");
        assert!(err.contains("--http.port"), "{err}");
        assert!(err.contains("--ws.port"), "{err}");
    }

    /// Cross-family wildcard overlap ([::] vs 0.0.0.0) depends on the platform's
    /// dual-stack configuration; it is deliberately left to the kernel to decide at bind.
    #[test]
    fn cross_family_wildcards_are_left_to_the_kernel() {
        let result = validate_rpc_addrs(
            addr("0.0.0.0:8545"),
            Some(addr("127.0.0.1:8551")),
            Some(addr("[::]:8545")),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn p2p_addr_unspecified_v4_announces_local_ip() {
        let local = ip("10.0.0.5");
        let (bind, ext) = resolve_p2p_endpoints(Some("0.0.0.0"), None, Some(local), None);
        assert_eq!(bind, ip("0.0.0.0"));
        assert_eq!(ext, local);
    }

    #[test]
    fn p2p_addr_unspecified_without_local_ip_keeps_unspecified() {
        let (bind, ext) = resolve_p2p_endpoints(Some("0.0.0.0"), None, None, None);
        assert_eq!(bind, ip("0.0.0.0"));
        assert_eq!(ext, ip("0.0.0.0"));
    }

    #[test]
    fn extip_overrides_unspecified_bind() {
        let (bind, ext) = resolve_p2p_endpoints(
            Some("0.0.0.0"),
            Some("203.0.113.5"),
            Some(ip("10.0.0.5")),
            None,
        );
        assert_eq!(bind, ip("0.0.0.0"));
        assert_eq!(ext, ip("203.0.113.5"));
    }

    #[test]
    fn specific_p2p_addr_used_for_both() {
        let (bind, ext) =
            resolve_p2p_endpoints(Some("10.0.0.5"), None, Some(ip("192.168.1.1")), None);
        assert_eq!(bind, ip("10.0.0.5"));
        assert_eq!(ext, ip("10.0.0.5"));
    }

    #[test]
    fn no_flags_uses_local_v4_when_available() {
        let local = ip("10.0.0.5");
        let (bind, ext) = resolve_p2p_endpoints(None, None, Some(local), Some(ip("fe80::1")));
        assert_eq!(bind, local);
        assert_eq!(ext, local);
    }

    #[test]
    fn extip_only_uses_unspecified_bind() {
        let (bind, ext) = resolve_p2p_endpoints(None, Some("203.0.113.5"), None, None);
        assert_eq!(bind, ip("0.0.0.0"));
        assert_eq!(ext, ip("203.0.113.5"));
    }

    #[test]
    fn p2p_addr_unspecified_v6_announces_local_ipv6() {
        let local6 = ip("fe80::1");
        let (bind, ext) = resolve_p2p_endpoints(Some("::"), None, None, Some(local6));
        assert_eq!(bind, ip("::"));
        assert_eq!(ext, local6);
    }

    #[test]
    #[should_panic(expected = "--p2p.addr and --nat.extip must use the same address family")]
    fn family_mismatch_panics() {
        let _ = resolve_p2p_endpoints(Some("0.0.0.0"), Some("::1"), None, None);
    }
}
