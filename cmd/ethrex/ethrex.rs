use bytes::Bytes;
use directories::ProjectDirs;
use ethrex_blockchain::{add_block, fork_choice::apply_fork_choice};
use ethrex_common::types::{Block, Genesis};
use ethrex_p2p::{
    kademlia::KademliaTable,
    network::{node_id_from_signing_key, peer_table},
    sync::{SyncManager, SyncMode},
    types::{Node, NodeRecord},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{backends::EVM, EVM_BACKEND};
use k256::ecdsa::SigningKey;
use local_ip_address::local_ip;
use rand::rngs::OsRng;
use std::{
    fs::{self, File},
    future::IntoFuture,
    io,
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs},
    path::{self, Path, PathBuf},
    str::FromStr as _,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};
mod cli;
mod decode;
mod networks;

const DEFAULT_DATADIR: &str = "ethrex";
#[tokio::main]
async fn main() {
    let matches = cli::cli().get_matches();

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
    let evm = EVM_BACKEND.get_or_init(|| evm.clone());
    info!("EVM_BACKEND set to: {:?}", evm);

    let path = path::PathBuf::from(data_dir.clone());
    let store: Store = if path.ends_with("memory") {
        Store::new(&data_dir, EngineType::InMemory).expect("Failed to create Store")
    } else {
        let engine_type;
        cfg_if::cfg_if! {
            if #[cfg(feature = "redb")] {
                engine_type = EngineType::RedB;
            } else if #[cfg(feature = "libmdbx")] {
                engine_type = EngineType::Libmdbx;
            } else {
                error!("No database specified. The feature flag `redb` or `libmdbx` should've been set while building.");
                panic!("Specify the desired database engine.");
            }
        }
        Store::new(&data_dir, engine_type).expect("Failed to create Store")
    };

    let genesis = read_genesis_file(&network);
    store
        .add_initial_state(genesis.clone())
        .expect("Failed to create genesis block");

    if let Some(chain_rlp_path) = matches.get_one::<String>("import") {
        info!("Importing blocks from chain file: {}", chain_rlp_path);
        let blocks = read_chain_file(chain_rlp_path);
        import_blocks(&store, &blocks);
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

        import_blocks(&store, &blocks);
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
    // Create SyncManager
    let syncer = SyncManager::new(peer_table.clone(), sync_mode);

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
    // We do not want to start the networking module if the l2 feature is enabled.
    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            if dev_mode {
                error!("Cannot run with DEV_MODE if the `l2` feature is enabled.");
                panic!("Run without the --dev argument.");
            }
            let l2_proposer = ethrex_l2::start_proposer(store).into_future();
            tracker.spawn(l2_proposer);
        } else if #[cfg(feature = "dev")] {
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
            store_known_peers(peer_table, peers_file).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            info!("Server shutting down!");
            return;
        }
    }
}

fn read_jwtsecret_file(jwt_secret_path: &str) -> Bytes {
    match File::open(jwt_secret_path) {
        Ok(mut file) => decode::jwtsecret_file(&mut file),
        Err(_) => write_jwtsecret_file(jwt_secret_path),
    }
}

fn write_jwtsecret_file(jwt_secret_path: &str) -> Bytes {
    info!("JWT secret not found in the provided path, generating JWT secret");
    let secret = generate_jwt_secret();
    std::fs::write(jwt_secret_path, &secret).expect("Unable to write JWT secret file");
    hex::decode(secret).unwrap().into()
}

fn generate_jwt_secret() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut secret = [0u8; 32];
    rng.fill(&mut secret);
    hex::encode(secret)
}

fn read_chain_file(chain_rlp_path: &str) -> Vec<Block> {
    let chain_file = std::fs::File::open(chain_rlp_path).expect("Failed to open chain rlp file");
    decode::chain_file(chain_file).expect("Failed to decode chain rlp file")
}

fn read_block_file(block_file_path: &str) -> Block {
    let encoded_block = std::fs::read(block_file_path)
        .unwrap_or_else(|_| panic!("Failed to read block file with path {}", block_file_path));
    Block::decode(&encoded_block)
        .unwrap_or_else(|_| panic!("Failed to decode block file {}", block_file_path))
}

fn read_genesis_file(genesis_file_path: &str) -> Genesis {
    let genesis_file = std::fs::File::open(genesis_file_path).expect("Failed to open genesis file");
    decode::genesis_file(genesis_file).expect("Failed to decode genesis file")
}

fn parse_socket_addr(addr: &str, port: &str) -> io::Result<SocketAddr> {
    // NOTE: this blocks until hostname can be resolved
    format!("{addr}:{port}")
        .to_socket_addrs()?
        .next()
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Failed to parse socket address",
        ))
}

fn sync_mode(matches: &clap::ArgMatches) -> SyncMode {
    let syncmode = matches.get_one::<String>("syncmode");
    match syncmode {
        Some(mode) if mode == "full" => SyncMode::Full,
        Some(mode) if mode == "snap" => SyncMode::Snap,
        other => panic!("Invalid syncmode {:?} expected either snap or full", other),
    }
}

fn set_datadir(datadir: &str) -> String {
    let project_dir = ProjectDirs::from("", "", datadir).expect("Couldn't find home directory");
    project_dir
        .data_local_dir()
        .to_str()
        .expect("invalid data directory")
        .to_owned()
}

fn import_blocks(store: &Store, blocks: &Vec<Block>) {
    let size = blocks.len();
    for block in blocks {
        let hash = block.hash();
        info!(
            "Adding block {} with hash {:#x}.",
            block.header.number, hash
        );
        let result = add_block(block, store);
        if let Some(error) = result.err() {
            warn!(
                "Failed to add block {} with hash {:#x}: {}.",
                block.header.number, hash, error
            );
        }
        if store
            .update_latest_block_number(block.header.number)
            .is_err()
        {
            error!("Fatal: added block {} but could not update the block number -- aborting block import", block.header.number);
            break;
        };
        if store
            .set_canonical_block(block.header.number, hash)
            .is_err()
        {
            error!(
                "Fatal: added block {} but could not set it as canonical -- aborting block import",
                block.header.number
            );
            break;
        };
    }
    if let Some(last_block) = blocks.last() {
        let hash = last_block.hash();
        match EVM_BACKEND.get() {
            Some(EVM::LEVM) => {
                // We are allowing this not to unwrap so that tests can run even if block execution results in the wrong root hash with LEVM.
                let _ = apply_fork_choice(store, hash, hash, hash);
            }
            // This means we are using REVM as default
            Some(EVM::REVM) | None => {
                apply_fork_choice(store, hash, hash, hash).unwrap();
            }
        }
    }
    info!("Added {} blocks to blockchain", size);
}

async fn store_known_peers(table: Arc<Mutex<KademliaTable>>, file_path: PathBuf) {
    let mut connected_peers = vec![];

    for peer in table.lock().await.iter_peers() {
        if peer.is_connected {
            connected_peers.push(peer.node.enode_url());
        }
    }

    let json = match serde_json::to_string(&connected_peers) {
        Ok(json) => json,
        Err(e) => {
            error!("Could not store peers in file: {:?}", e);
            return;
        }
    };

    if let Err(e) = std::fs::write(file_path, json) {
        error!("Could not store peers in file: {:?}", e);
    };
}

fn read_known_peers(file_path: PathBuf) -> Result<Vec<Node>, serde_json::Error> {
    let Ok(file) = std::fs::File::open(file_path) else {
        return Ok(vec![]);
    };

    serde_json::from_reader(file)
}
