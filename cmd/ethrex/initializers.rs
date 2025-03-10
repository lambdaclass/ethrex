use crate::{
    networks,
    utils::{
        parse_socket_addr, read_genesis_file, read_jwtsecret_file, read_known_peers, sync_mode,
    },
};
use bytes::Bytes;
use clap::ArgMatches;
use ethrex_blockchain::Blockchain;
use ethrex_p2p::{
    kademlia::KademliaTable,
    network::node_id_from_signing_key,
    sync::SyncManager,
    types::{Node, NodeRecord},
};
#[cfg(feature = "based")]
use ethrex_rpc::{EngineClient, EthClient};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::backends::EvmEngine;
use k256::ecdsa::SigningKey;
use local_ip_address::local_ip;
use rand::rngs::OsRng;
use std::{
    fs,
    future::IntoFuture,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info, warn};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

pub fn init_tracing(matches: &ArgMatches) {
    let log_level = matches
        .get_one::<String>("log.level")
        .expect("shouldn't happen, log.level is used with a default value");
    let log_filter = EnvFilter::builder()
        .with_default_directive(
            Directive::from_str(log_level).expect("Not supported log level provided"),
        )
        .from_env_lossy();
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(log_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

pub fn init_metrics(matches: &ArgMatches, tracker: TaskTracker) {
    // Check if the metrics.port is present, else set it to 0
    let metrics_port = matches
        .get_one::<String>("metrics.port")
        .map_or("0".to_owned(), |v| v.clone());

    // Start the metrics_api with the given metrics.port if it's != 0
    if metrics_port != *"0" {
        let metrics_api = ethrex_metrics::api::start_prometheus_metrics_api(metrics_port);
        tracker.spawn(metrics_api);
    }
}

pub fn init_store(data_dir: &str, network: &str) -> Store {
    let path = PathBuf::from(data_dir);
    let store = if path.ends_with("memory") {
        Store::new(data_dir, EngineType::InMemory).expect("Failed to create Store")
    } else {
        cfg_if::cfg_if! {
            if #[cfg(feature = "redb")] {
                let engine_type = EngineType::RedB;
            } else if #[cfg(feature = "libmdbx")] {
                let engine_type = EngineType::Libmdbx;
            } else {
                let engine_type = EngineType::InMemory;
                error!("No database specified. The feature flag `redb` or `libmdbx` should've been set while building.");
                panic!("Specify the desired database engine.");
            }
        }
        Store::new(data_dir, engine_type).expect("Failed to create Store")
    };
    let genesis = read_genesis_file(network);
    store
        .add_initial_state(genesis.clone())
        .expect("Failed to create genesis block");
    store
}

pub fn init_blockchain(evm_engine: EvmEngine, store: Store) -> Arc<Blockchain> {
    Blockchain::new(evm_engine, store).into()
}

#[allow(clippy::too_many_arguments)]
pub fn init_rpc_api(
    matches: &ArgMatches,
    signer: &SigningKey,
    peer_table: Arc<Mutex<KademliaTable>>,
    local_p2p_node: Node,
    store: Store,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    tracker: TaskTracker,
) {
    let enr_seq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let local_node_record = NodeRecord::from_node(local_p2p_node, enr_seq, signer)
        .expect("Node record could not be created from local node");

    // Create SyncManager
    let syncer = SyncManager::new(
        peer_table.clone(),
        sync_mode(matches),
        cancel_token,
        blockchain.clone(),
    );

    let rpc_api = ethrex_rpc::start_api(
        get_http_socket_addr(matches),
        get_authrpc_socket_addr(matches),
        store,
        blockchain,
        get_jwt_secret(matches),
        local_p2p_node,
        local_node_record,
        syncer,
        #[cfg(feature = "based")]
        get_gateway_http_client(matches),
        #[cfg(feature = "based")]
        get_gateway_auth_client(matches),
    )
    .into_future();

    tracker.spawn(rpc_api);
}

#[cfg(feature = "based")]
fn get_gateway_http_client(matches: &clap::ArgMatches) -> EthClient {
    let gateway_addr = matches
        .get_one::<String>("gateway.addr")
        .expect("gateway.addr is required");
    let gateway_eth_port = matches
        .get_one::<String>("gateway.eth_port")
        .expect("gateway.eth_port is required");

    let gateway_http_socket_addr = parse_socket_addr(gateway_addr, gateway_eth_port)
        .expect("Failed to parse gateway http address and port");

    EthClient::new(&gateway_http_socket_addr.to_string())
}

#[cfg(feature = "based")]
fn get_gateway_auth_client(matches: &clap::ArgMatches) -> EngineClient {
    let gateway_addr = matches
        .get_one::<String>("gateway.addr")
        .expect("gateway.addr is required");
    let gateway_auth_port = matches
        .get_one::<String>("gateway.auth_port")
        .expect("gateway.auth_port is required");
    let gateway_authrpc_jwtsecret = matches
        .get_one::<String>("gateway.jwtsecret")
        .expect("gateway.jwtsecret is required");

    let gateway_authrpc_socket_addr = parse_socket_addr(gateway_addr, gateway_auth_port)
        .expect("Failed to parse gateway authrpc address and port");

    let gateway_jwtsecret = read_jwtsecret_file(gateway_authrpc_jwtsecret);

    EngineClient::new(&gateway_authrpc_socket_addr.to_string(), gateway_jwtsecret)
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub async fn init_network(
    matches: &ArgMatches,
    network: &str,
    data_dir: &str,
    local_p2p_node: Node,
    signer: SigningKey,
    peer_table: Arc<Mutex<KademliaTable>>,
    store: Store,
    tracker: TaskTracker,
    blockchain: Arc<Blockchain>,
) {
    let dev_mode = *matches.get_one::<bool>("dev").unwrap_or(&false);

    if dev_mode {
        error!("Binary wasn't built with The feature flag `dev` enabled.");
        panic!(
            "Build the binary with the `dev` feature in order to use the `--dev` cli's argument."
        );
    }

    let bootnodes = get_bootnodes(matches, network, data_dir);

    ethrex_p2p::start_network(
        local_p2p_node,
        tracker.clone(),
        bootnodes,
        signer,
        peer_table.clone(),
        store,
        blockchain,
    )
    .await
    .expect("Network starts");

    tracker.spawn(ethrex_p2p::periodically_show_peer_stats(peer_table.clone()));
}

#[cfg(feature = "dev")]
pub fn init_dev_network(matches: &ArgMatches, store: &Store, tracker: TaskTracker) {
    let dev_mode = *matches.get_one::<bool>("dev").unwrap_or(&false);

    if dev_mode {
        info!("Running in DEV_MODE");

        let head_block_hash = {
            let current_block_number = store.get_latest_block_number().unwrap();
            store
                .get_canonical_block_hash(current_block_number)
                .unwrap()
                .unwrap()
        };

        let max_tries = 3;

        let url = format!(
            "http://{authrpc_socket_addr}",
            authrpc_socket_addr = get_authrpc_socket_addr(matches)
        );

        let block_producer_engine = ethrex_dev::block_producer::start_block_producer(
            url,
            get_jwt_secret(matches),
            head_block_hash,
            max_tries,
            1000,
            ethrex_common::Address::default(),
        );
        tracker.spawn(block_producer_engine);
    }
}

pub fn get_network(matches: &ArgMatches) -> String {
    let mut network = matches
        .get_one::<String>("network")
        .expect("network is required")
        .clone();

    // Set preset genesis from known networks
    if network == "holesky" {
        network = String::from(networks::HOLESKY_GENESIS_PATH);
    }
    if network == "sepolia" {
        network = String::from(networks::SEPOLIA_GENESIS_PATH);
    }
    if network == "mekong" {
        network = String::from(networks::MEKONG_GENESIS_PATH);
    }
    if network == "ephemery" {
        network = String::from(networks::EPHEMERY_GENESIS_PATH);
    }

    network
}

#[allow(dead_code)]
pub fn get_bootnodes(matches: &ArgMatches, network: &str, data_dir: &str) -> Vec<Node> {
    let mut bootnodes: Vec<Node> = matches
        .get_many("bootnodes")
        .map(Iterator::copied)
        .map(Iterator::collect)
        .unwrap_or_default();

    if network == networks::HOLESKY_GENESIS_PATH {
        info!("Adding holesky preset bootnodes");
        bootnodes.extend(dbg!(networks::HOLESKY_BOOTNODES.iter()));
    }

    if network == networks::SEPOLIA_GENESIS_PATH {
        info!("Adding sepolia preset bootnodes");
        bootnodes.extend(networks::SEPOLIA_BOOTNODES.iter());
    }

    if network == networks::MEKONG_GENESIS_PATH {
        info!("Adding mekong preset bootnodes");
        bootnodes.extend(networks::MEKONG_BOOTNODES.iter());
    }

    if network == networks::EPHEMERY_GENESIS_PATH {
        info!("Adding ephemery preset bootnodes");
        bootnodes.extend(networks::EPHEMERY_BOOTNODES.iter());
    }

    if bootnodes.is_empty() {
        warn!("No bootnodes specified. This node will not be able to connect to the network.");
    }

    let peers_file = PathBuf::from(data_dir.to_owned() + "/peers.json");

    info!("Reading known peers from {:?}", peers_file);

    match read_known_peers(peers_file.clone()) {
        Ok(ref mut known_peers) => bootnodes.append(known_peers),
        Err(e) => error!("Could not read from peers file: {e}"),
    };

    bootnodes
}

pub fn get_signer(data_dir: &str) -> SigningKey {
    // Get the signer from the default directory, create one if the key file is not present.
    let key_path = Path::new(data_dir).join("node.key");
    let signer = match fs::read(key_path.clone()) {
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
    };
    signer
}

pub fn get_local_p2p_node(matches: &ArgMatches, signer: &SigningKey) -> Node {
    let udp_addr = matches
        .get_one::<String>("discovery.addr")
        .expect("discovery.addr is required");
    let udp_port = matches
        .get_one::<String>("discovery.port")
        .expect("discovery.port is required");
    let udp_socket_addr =
        parse_socket_addr(udp_addr, udp_port).expect("Failed to parse discovery address and port");

    let tcp_addr = matches
        .get_one::<String>("p2p.addr")
        .expect("addr is required");
    let tcp_port = matches
        .get_one::<String>("p2p.port")
        .expect("port is required");
    let tcp_socket_addr =
        parse_socket_addr(tcp_addr, tcp_port).expect("Failed to parse addr and port");

    // TODO: If hhtp.addr is 0.0.0.0 we get the local ip as the one of the node, otherwise we use the provided one.
    // This is fine for now, but we might need to support more options in the future.
    let p2p_node_ip = if udp_socket_addr.ip() == Ipv4Addr::new(0, 0, 0, 0) {
        local_ip().expect("Failed to get local ip")
    } else {
        udp_socket_addr.ip()
    };

    let local_node_id = node_id_from_signing_key(signer);

    let node = Node {
        ip: p2p_node_ip,
        udp_port: udp_socket_addr.port(),
        tcp_port: tcp_socket_addr.port(),
        node_id: local_node_id,
    };

    // TODO Find a proper place to show node information
    // https://github.com/lambdaclass/ethrex/issues/836
    let enode = node.enode_url();
    info!("Node: {enode}");

    node
}

pub fn get_jwt_secret(matches: &ArgMatches) -> Bytes {
    let authrpc_jwtsecret = matches
        .get_one::<String>("authrpc.jwtsecret")
        .expect("authrpc.jwtsecret is required");
    read_jwtsecret_file(authrpc_jwtsecret)
}

pub fn get_authrpc_socket_addr(matches: &ArgMatches) -> SocketAddr {
    let authrpc_addr = matches
        .get_one::<String>("authrpc.addr")
        .expect("authrpc.addr is required");
    let authrpc_port = matches
        .get_one::<String>("authrpc.port")
        .expect("authrpc.port is required");
    parse_socket_addr(authrpc_addr, authrpc_port).expect("Failed to parse authrpc address and port")
}

pub fn get_http_socket_addr(matches: &ArgMatches) -> SocketAddr {
    let http_addr = matches
        .get_one::<String>("http.addr")
        .expect("http.addr is required");
    let http_port = matches
        .get_one::<String>("http.port")
        .expect("http.port is required");
    parse_socket_addr(http_addr, http_port).expect("Failed to parse http address and port")
}
