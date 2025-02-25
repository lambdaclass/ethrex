use std::{
    fs,
    future::IntoFuture,
    net::Ipv4Addr,
    path::{self, Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use crate::launch::DEFAULT_DATADIR;
use crate::networks;
use crate::utils::{
    parse_socket_addr, read_block_file, read_chain_file, read_genesis_file, read_jwtsecret_file,
    read_known_peers, set_datadir, store_known_peers, sync_mode,
};
use ethrex_blockchain::Blockchain;
use ethrex_p2p::{
    network::{node_id_from_signing_key, peer_table},
    sync::SyncManager,
    types::{Node, NodeRecord},
};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::backends::EVM;
use k256::ecdsa::SigningKey;
use local_ip_address::local_ip;
use rand::rngs::OsRng;
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

pub async fn launch(matches: clap::ArgMatches) {
    if let Some(matches) = matches.subcommand_matches("removedb") {
        let data_dir = matches
            .get_one::<String>("datadir")
            .map_or(set_datadir(DEFAULT_DATADIR), |datadir| set_datadir(datadir));
        let path = Path::new(&data_dir);
        if path.exists() {
            std::fs::remove_dir_all(path).expect("Failed to remove data directory");
        } else {
            warn!("Data directory does not exist: {}", data_dir);
        }
        return;
    }

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

    let http_addr = matches
        .get_one::<String>("http.addr")
        .expect("http.addr is required");
    let http_port = matches
        .get_one::<String>("http.port")
        .expect("http.port is required");
    let authrpc_addr = matches
        .get_one::<String>("authrpc.addr")
        .expect("authrpc.addr is required");
    let authrpc_port = matches
        .get_one::<String>("authrpc.port")
        .expect("authrpc.port is required");
    let authrpc_jwtsecret = matches
        .get_one::<String>("authrpc.jwtsecret")
        .expect("authrpc.jwtsecret is required");

    let tcp_addr = matches
        .get_one::<String>("p2p.addr")
        .expect("addr is required");
    let tcp_port = matches
        .get_one::<String>("p2p.port")
        .expect("port is required");
    let udp_addr = matches
        .get_one::<String>("discovery.addr")
        .expect("discovery.addr is required");
    let udp_port = matches
        .get_one::<String>("discovery.port")
        .expect("discovery.port is required");

    let mut network = matches
        .get_one::<String>("network")
        .expect("network is required")
        .clone();

    let mut bootnodes: Vec<Node> = matches
        .get_many("bootnodes")
        .map(Iterator::copied)
        .map(Iterator::collect)
        .unwrap_or_default();

    if network == "holesky" {
        info!("Adding holesky preset bootnodes");
        // Set holesky presets
        network = String::from(networks::HOLESKY_GENESIS_PATH);
        bootnodes.extend(networks::HOLESKY_BOOTNODES.iter());
    }

    if network == "sepolia" {
        info!("Adding sepolia preset bootnodes");
        // Set sepolia presets
        network = String::from(networks::SEPOLIA_GENESIS_PATH);
        bootnodes.extend(networks::SEPOLIA_BOOTNODES.iter());
    }

    if network == "mekong" {
        info!("Adding mekong preset bootnodes");
        // Set mekong presets
        network = String::from(networks::MEKONG_GENESIS_PATH);
        bootnodes.extend(networks::MEKONG_BOOTNODES.iter());
    }

    if bootnodes.is_empty() {
        warn!("No bootnodes specified. This node will not be able to connect to the network.");
    }

    let http_socket_addr =
        parse_socket_addr(http_addr, http_port).expect("Failed to parse http address and port");
    let authrpc_socket_addr = parse_socket_addr(authrpc_addr, authrpc_port)
        .expect("Failed to parse authrpc address and port");

    let udp_socket_addr =
        parse_socket_addr(udp_addr, udp_port).expect("Failed to parse discovery address and port");
    let tcp_socket_addr =
        parse_socket_addr(tcp_addr, tcp_port).expect("Failed to parse addr and port");

    let data_dir = matches
        .get_one::<String>("datadir")
        .map_or(set_datadir(DEFAULT_DATADIR), |datadir| set_datadir(datadir));

    let peers_file = PathBuf::from(data_dir.clone() + "/peers.json");
    info!("Reading known peers from {:?}", peers_file);
    match read_known_peers(peers_file.clone()) {
        Ok(ref mut known_peers) => bootnodes.append(known_peers),
        Err(e) => error!("Could not read from peers file: {}", e),
    };

    let sync_mode = sync_mode(&matches);

    let evm = matches.get_one::<EVM>("evm").unwrap_or(&EVM::REVM);

    let path = path::PathBuf::from(data_dir.clone());
    let store: Store = if path.ends_with("memory") {
        Store::new(&data_dir, EngineType::InMemory).expect("Failed to create Store")
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
        Store::new(&data_dir, engine_type).expect("Failed to create Store")
    };
    let blockchain = Blockchain::new(evm.clone(), store.clone());

    let genesis = read_genesis_file(&network);
    store
        .add_initial_state(genesis.clone())
        .expect("Failed to create genesis block");

    if let Some(chain_rlp_path) = matches.get_one::<String>("import") {
        info!("Importing blocks from chain file: {}", chain_rlp_path);
        let blocks = read_chain_file(chain_rlp_path);
        blockchain.import_blocks(&blocks);
    }

    if let Some(blocks_path) = matches.get_one::<String>("import_dir") {
        info!(
            "Importing blocks from individual block files in directory: {}",
            blocks_path
        );
        let mut blocks = vec![];
        let dir_reader = fs::read_dir(blocks_path).expect("Failed to read blocks directory");
        for file_res in dir_reader {
            let file = file_res.expect("Failed to open file in directory");
            let path = file.path();
            let s = path
                .to_str()
                .expect("Path could not be converted into string");
            blocks.push(read_block_file(s));
        }

        blockchain.import_blocks(&blocks);
    }

    let jwt_secret = read_jwtsecret_file(authrpc_jwtsecret);

    // Get the signer from the default directory, create one if the key file is not present.
    let key_path = Path::new(&data_dir).join("node.key");
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

    let local_node_id = node_id_from_signing_key(&signer);

    // TODO: If hhtp.addr is 0.0.0.0 we get the local ip as the one of the node, otherwise we use the provided one.
    // This is fine for now, but we might need to support more options in the future.
    let p2p_node_ip = if udp_socket_addr.ip() == Ipv4Addr::new(0, 0, 0, 0) {
        local_ip().expect("Failed to get local ip")
    } else {
        udp_socket_addr.ip()
    };

    let local_p2p_node = Node {
        ip: p2p_node_ip,
        udp_port: udp_socket_addr.port(),
        tcp_port: tcp_socket_addr.port(),
        node_id: local_node_id,
    };
    let enr_seq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let local_node_record = NodeRecord::from_node(local_p2p_node, enr_seq, &signer)
        .expect("Node record could not be created from local node");
    // Create Kademlia Table here so we can access it from rpc server (for syncing)
    let peer_table = peer_table(signer.clone());
    // Create a cancellation_token for long_living tasks
    let cancel_token = tokio_util::sync::CancellationToken::new();
    // Create SyncManager
    let syncer = SyncManager::new(
        peer_table.clone(),
        sync_mode,
        cancel_token.clone(),
        blockchain,
    );

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();
    let rpc_api = ethrex_rpc::start_api(
        http_socket_addr,
        authrpc_socket_addr,
        store.clone(),
        jwt_secret,
        local_p2p_node,
        local_node_record,
        syncer,
    )
    .into_future();

    // TODO Find a proper place to show node information
    // https://github.com/lambdaclass/ethrex/issues/836
    let enode = local_p2p_node.enode_url();
    info!("Node: {enode}");

    tracker.spawn(rpc_api);

    // Check if the metrics.port is present, else set it to 0
    let metrics_port = matches
        .get_one::<String>("metrics.port")
        .map_or("0".to_owned(), |v| v.clone());

    // Start the metrics_api with the given metrics.port if it's != 0
    if metrics_port != *"0" {
        let metrics_api = ethrex_metrics::api::start_prometheus_metrics_api(metrics_port);
        tracker.spawn(metrics_api);
    }

    let dev_mode = *matches.get_one::<bool>("dev").unwrap_or(&false);
    cfg_if::cfg_if! {
        if #[cfg(feature = "dev")] {
            use ethrex_dev;
            // Start the block_producer module if devmode was set
            if dev_mode {
                info!("Runnning in DEV_MODE");
                let authrpc_jwtsecret =
                    std::fs::read(authrpc_jwtsecret).expect("Failed to read JWT secret");
                let head_block_hash = {
                    let current_block_number = store.get_latest_block_number().unwrap();
                    store
                        .get_canonical_block_hash(current_block_number)
                        .unwrap()
                        .unwrap()
                };
                let max_tries = 3;
                let url = format!("http://{authrpc_socket_addr}");
                let block_producer_engine = ethrex_dev::block_producer::start_block_producer(
                    url,
                    authrpc_jwtsecret.into(),
                    head_block_hash,
                    max_tries,
                    1000,
                    ethrex_common::Address::default(),
                );
                tracker.spawn(block_producer_engine);
            }
        } else {
            if dev_mode {
                error!("Binary wasn't built with The feature flag `dev` enabled.");
                panic!("Build the binary with the `dev` feature in order to use the `--dev` cli's argument.");
            }
            ethrex_p2p::start_network(
                local_p2p_node,
                tracker.clone(),
                bootnodes,
                signer,
                peer_table.clone(),
                store,
            )
            .await.expect("Network starts");
            tracker.spawn(ethrex_p2p::periodically_show_peer_stats(peer_table.clone()));
        }
    }
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Server shut down started...");
            info!("Storing known peers at {:?}...", peers_file);
            cancel_token.cancel();
            store_known_peers(peer_table, peers_file).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            info!("Server shutting down!");
        }
    }
}
