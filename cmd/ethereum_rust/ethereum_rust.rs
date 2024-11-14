use bytes::Bytes;
use directories::ProjectDirs;
use ethereum_rust_blockchain::add_block;
use ethereum_rust_blockchain::fork_choice::apply_fork_choice;
use ethereum_rust_core::types::{Block, Genesis};
use ethereum_rust_core::H256;
use ethereum_rust_net::bootnode::BootNode;
use ethereum_rust_net::node_id_from_signing_key;
use ethereum_rust_net::types::Node;
use ethereum_rust_storage::{EngineType, Store};
use k256::ecdsa::SigningKey;
use local_ip_address::local_ip;
use std::future::IntoFuture;
use std::path::Path;
use std::str::FromStr as _;
use std::time::Duration;
use std::{
    fs::File,
    io,
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs},
};
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};
use tracing_subscriber::filter::Directive;
use tracing_subscriber::{EnvFilter, FmtSubscriber};
mod cli;
mod decode;

const DEFAULT_DATADIR: &str = "ethereum_rust";
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

    let genesis_file_path = matches
        .get_one::<String>("network")
        .expect("network is required");

    let bootnodes: Vec<BootNode> = matches
        .get_many("bootnodes")
        .map(Iterator::copied)
        .map(Iterator::collect)
        .unwrap_or_default();

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

    let store = Store::new(&data_dir, EngineType::Libmdbx).expect("Failed to create Store");

    let genesis = read_genesis_file(genesis_file_path);
    store
        .add_initial_state(genesis.clone())
        .expect("Failed to create genesis block");

    if let Some(chain_rlp_path) = matches.get_one::<String>("import") {
        let blocks = read_chain_file(chain_rlp_path);
        let size = blocks.len();
        for block in &blocks {
            let hash = block.hash();
            info!(
                "Adding block {} with hash {:#x}.",
                block.header.number, hash
            );
            let result = add_block(block, &store);
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
                error!("Fatal: added block {} but could not set it as canonical -- aborting block import", block.header.number);
                break;
            };
        }
        if let Some(last_block) = blocks.last() {
            let hash = last_block.hash();
            apply_fork_choice(&store, hash, hash, hash).unwrap();
        }
        info!("Added {} blocks to blockchain", size);
    }
    let jwt_secret = read_jwtsecret_file(authrpc_jwtsecret);

    // TODO Learn how should the key be created
    // https://github.com/lambdaclass/lambda_ethereum_rust/issues/836
    //let signer = SigningKey::random(&mut OsRng);
    let key_bytes =
        H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3").unwrap();
    let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();
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

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();
    let rpc_api = ethereum_rust_rpc::start_api(
        http_socket_addr,
        authrpc_socket_addr,
        store.clone(),
        jwt_secret,
        local_p2p_node,
    )
    .into_future();

    // TODO Find a proper place to show node information
    // https://github.com/lambdaclass/lambda_ethereum_rust/issues/836
    let enode = local_p2p_node.enode_url();
    info!("Node: {enode}");

    tracker.spawn(rpc_api);

    // We do not want to start the networking module if the l2 feature is enabled.
    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            let l2_proposer = ethereum_rust_l2::start_proposer(store).into_future();
            tracker.spawn(l2_proposer);
        } else if #[cfg(feature = "dev")] {
            use ethereum_rust_dev;

            let authrpc_jwtsecret = std::fs::read(authrpc_jwtsecret).expect("Failed to read JWT secret");
            let head_block_hash = {
                let current_block_number = store.get_latest_block_number().unwrap().unwrap();
                store.get_canonical_block_hash(current_block_number).unwrap().unwrap()
            };
            let max_tries = 3;
            let block_producer_engine = ethereum_rust_dev::block_producer::start_block_producer(authrpc_socket_addr, authrpc_jwtsecret.into(), head_block_hash, max_tries);
            tracker.spawn(block_producer_engine);
        } else {
            let networking = ethereum_rust_net::start_network(
                udp_socket_addr,
                tcp_socket_addr,
                bootnodes,
                signer,
                store,
            )
            .into_future();
            tracker.spawn(networking);
        }
    }

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Server shut down started...");
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

fn set_datadir(datadir: &str) -> String {
    let project_dir = ProjectDirs::from("", "", datadir).expect("Couldn't find home directory");
    project_dir
        .data_local_dir()
        .to_str()
        .expect("invalid data directory")
        .to_owned()
}
