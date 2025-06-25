use crate::{
    cli::Options,
    networks::{self, Network, PublicNetwork},
    utils::{get_client_version, parse_socket_addr, read_jwtsecret_file, read_node_config_file},
};
use ethrex_blockchain::Blockchain;
use ethrex_common::types::Genesis;
use ethrex_p2p::{
    kademlia::KademliaTable,
    network::{P2PContext, public_key_from_signing_key},
    peer_handler::PeerHandler,
    sync_manager::SyncManager,
    types::{Node, NodeRecord},
};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmEngine;
use k256::ecdsa::SigningKey;
use local_ip_address::local_ip;
use rand::rngs::OsRng;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber, filter::Directive};

#[cfg(feature = "l2")]
use crate::l2::L2Options;
#[cfg(feature = "l2")]
use ::{
    ethrex_common::Address,
    ethrex_storage_rollup::{EngineTypeRollup, StoreRollup},
};

pub fn init_tracing(opts: &Options) {
    let log_filter = EnvFilter::builder()
        .with_default_directive(Directive::from(opts.log_level))
        .from_env_lossy();
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(log_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
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
    tracker.spawn(metrics_api);
}

/// Opens a New or Pre-exsisting Store and loads the initial state provided by the network
pub async fn init_store(data_dir: &str, genesis: Genesis) -> Store {
    let store = open_store(data_dir);
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to create genesis block");
    store
}

/// Opens a Pre-exsisting Store or creates a new one
pub fn open_store(data_dir: &str) -> Store {
    let path = PathBuf::from(data_dir);
    if path.ends_with("memory") {
        Store::new(data_dir, EngineType::InMemory).expect("Failed to create Store")
    } else {
        cfg_if::cfg_if! {
            if #[cfg(feature = "redb")] {
                let engine_type = EngineType::RedB;
            } else if #[cfg(feature = "libmdbx")] {
                let engine_type = EngineType::Libmdbx;
            } else {
                error!("No database specified. The feature flag `redb` or `libmdbx` should've been set while building.");
                panic!("Specify the desired database engine.");
            }
        }
        Store::new(data_dir, engine_type).expect("Failed to create Store")
    }
}

#[cfg(feature = "l2")]
pub async fn init_rollup_store(data_dir: &str) -> StoreRollup {
    cfg_if::cfg_if! {
        if #[cfg(feature = "rollup_storage_sql")] {
            let engine_type = EngineTypeRollup::SQL;
        } else if #[cfg(feature = "rollup_storage_redb")] {
            let engine_type = EngineTypeRollup::RedB;
        } else if #[cfg(feature = "rollup_storage_libmdbx")] {
            let engine_type = EngineTypeRollup::Libmdbx;
        } else {
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

pub fn init_blockchain(evm_engine: EvmEngine, store: Store) -> Arc<Blockchain> {
    info!("Initiating blockchain with EVM: {}", evm_engine);
    Blockchain::new(evm_engine, store).into()
}

#[allow(clippy::too_many_arguments)]
pub async fn init_rpc_api(
    opts: &Options,
    #[cfg(feature = "l2")] l2_opts: &L2Options,
    peer_table: Arc<Mutex<KademliaTable>>,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    store: Store,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    tracker: TaskTracker,
    #[cfg(feature = "l2")] rollup_store: StoreRollup,
) {
    let peer_handler = PeerHandler::new(peer_table);

    // Create SyncManager
    let syncer = SyncManager::new(
        peer_handler.clone(),
        opts.syncmode.clone(),
        cancel_token,
        blockchain.clone(),
        store.clone(),
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
        #[cfg(feature = "l2")]
        get_valid_delegation_addresses(l2_opts),
        #[cfg(feature = "l2")]
        l2_opts.sponsor_private_key,
        #[cfg(feature = "l2")]
        rollup_store,
    );

    tracker.spawn(rpc_api);
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub async fn init_network(
    opts: &Options,
    network: &Network,
    data_dir: &str,
    local_p2p_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SigningKey,
    peer_table: Arc<Mutex<KademliaTable>>,
    store: Store,
    tracker: TaskTracker,
    blockchain: Arc<Blockchain>,
) {
    if opts.dev {
        error!("Binary wasn't built with The feature flag `dev` enabled.");
        panic!(
            "Build the binary with the `dev` feature in order to use the `--dev` cli's argument."
        );
    }

    let bootnodes = get_bootnodes(opts, network, data_dir);

    let context = P2PContext::new(
        local_p2p_node,
        local_node_record,
        tracker.clone(),
        signer,
        peer_table.clone(),
        store,
        blockchain,
        get_client_version(),
    );

    context.set_fork_id().await.expect("Set fork id");

    ethrex_p2p::start_network(context, bootnodes)
        .await
        .expect("Network starts");

    tracker.spawn(ethrex_p2p::periodically_show_peer_stats(peer_table.clone()));
}

#[cfg(feature = "dev")]
pub async fn init_dev_network(opts: &Options, store: &Store, tracker: TaskTracker) {
    if opts.dev {
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
}

pub fn get_network(opts: &Options) -> Network {
    opts.network.clone()
}

#[allow(dead_code)]
pub fn get_bootnodes(opts: &Options, network: &Network, data_dir: &str) -> Vec<Node> {
    let mut bootnodes: Vec<Node> = opts.bootnodes.clone();

    match network {
        Network::PublicNetwork(PublicNetwork::Holesky) => {
            info!("Adding holesky preset bootnodes");
            bootnodes.extend(networks::HOLESKY_BOOTNODES.clone());
        }
        Network::PublicNetwork(PublicNetwork::Hoodi) => {
            info!("Addig hoodi preset bootnodes");
            bootnodes.extend(networks::HOODI_BOOTNODES.clone());
        }
        Network::PublicNetwork(PublicNetwork::Mainnet) => {
            info!("Adding mainnet preset bootnodes");
            bootnodes.extend(networks::MAINNET_BOOTNODES.clone());
        }
        Network::PublicNetwork(PublicNetwork::Sepolia) => {
            info!("Adding sepolia preset bootnodes");
            bootnodes.extend(networks::SEPOLIA_BOOTNODES.clone());
        }
        _ => {}
    }

    if bootnodes.is_empty() {
        warn!("No bootnodes specified. This node will not be able to connect to the network.");
    }

    let config_file = PathBuf::from(data_dir.to_owned() + "/node_config.json");

    info!("Reading known peers from config file {:?}", config_file);

    match read_node_config_file(config_file) {
        Ok(ref mut config) => bootnodes.append(&mut config.known_peers),
        Err(e) => error!("Could not read from peers file: {e}"),
    };

    bootnodes
}

pub fn get_signer(data_dir: &str) -> SigningKey {
    // Get the signer from the default directory, create one if the key file is not present.
    let key_path = Path::new(data_dir).join("node.key");
    match fs::read(key_path.clone()) {
        Ok(content) => SigningKey::from_slice(&content).expect("Signing key could not be created."),
        Err(_) => {
            info!(
                "Key file not found, creating a new key and saving to {:?}",
                key_path
            );
            if let Some(parent) = key_path.parent() {
                fs::create_dir_all(parent).expect("Key file path could not be created.")
            }
            let signer = SigningKey::random(&mut OsRng);
            fs::write(key_path, signer.to_bytes())
                .expect("Newly created signer could not be saved to disk.");
            signer
        }
    }
}

pub fn get_local_p2p_node(opts: &Options, signer: &SigningKey) -> Node {
    let udp_socket_addr = parse_socket_addr(&opts.discovery_addr, &opts.discovery_port)
        .expect("Failed to parse discovery address and port");
    let tcp_socket_addr =
        parse_socket_addr(&opts.p2p_addr, &opts.p2p_port).expect("Failed to parse addr and port");

    // TODO: If hhtp.addr is 0.0.0.0 we get the local ip as the one of the node, otherwise we use the provided one.
    // This is fine for now, but we might need to support more options in the future.
    let p2p_node_ip = if udp_socket_addr.ip() == Ipv4Addr::new(0, 0, 0, 0) {
        local_ip().expect("Failed to get local ip")
    } else {
        udp_socket_addr.ip()
    };

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
    info!("Node: {enode}");

    node
}

pub fn get_local_node_record(
    data_dir: &str,
    local_p2p_node: &Node,
    signer: &SigningKey,
) -> NodeRecord {
    let config_file = PathBuf::from(data_dir.to_owned() + "/node_config.json");

    match read_node_config_file(config_file) {
        Ok(ref mut config) => {
            NodeRecord::from_node(local_p2p_node, config.node_record.seq + 1, signer)
                .expect("Node record could not be created from local node")
        }
        Err(_) => {
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

#[cfg(feature = "l2")]
pub fn get_valid_delegation_addresses(l2_opts: &L2Options) -> Vec<Address> {
    let Some(ref path) = l2_opts.sponsorable_addresses_file_path else {
        warn!("No valid addresses provided, ethrex_SendTransaction will always fail");
        return Vec::new();
    };
    let addresses: Vec<Address> = fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("Failed to load file {}", path))
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
