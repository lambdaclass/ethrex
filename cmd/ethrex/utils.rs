use crate::decode;
use bytes::Bytes;
use directories::ProjectDirs;
use ethrex_common::types::{Block, Genesis};
use ethrex_p2p::{
    peer_table::PeerTable,
    sync::SyncMode,
    types::{Node, NodeRecord},
};
use hex::FromHexError;
use secp256k1::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
};
use tracing::{error, info};

#[derive(Serialize, Deserialize)]
pub struct NodeConfigFile {
    pub known_peers: Vec<Node>,
    pub node_record: NodeRecord,
}

impl NodeConfigFile {
    pub async fn new(mut peer_table: PeerTable, node_record: NodeRecord) -> Self {
        let connected_peers = peer_table.get_connected_nodes().await.unwrap_or(Vec::new());

        NodeConfigFile {
            known_peers: connected_peers,
            node_record,
        }
    }
}

pub fn read_jwtsecret_file(jwt_secret_path: &str) -> Bytes {
    match File::open(jwt_secret_path) {
        Ok(mut file) => decode::jwtsecret_file(&mut file),
        Err(_) => write_jwtsecret_file(jwt_secret_path),
    }
}

pub fn write_jwtsecret_file(jwt_secret_path: &str) -> Bytes {
    info!("JWT secret not found in the provided path, generating JWT secret");
    let secret = generate_jwt_secret();

    // Ensure the directory exists
    if let Some(parent_dir) = Path::new(jwt_secret_path).parent() {
        std::fs::create_dir_all(parent_dir).expect("Failed to create parent dir");
    }

    std::fs::write(jwt_secret_path, &secret).expect("Unable to write JWT secret file");
    hex::decode(secret)
        .map(Bytes::from)
        .expect("Failed to decode generated JWT secret")
}

pub fn generate_jwt_secret() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut secret = [0u8; 32];
    rng.fill(&mut secret);
    hex::encode(secret)
}

pub fn read_chain_file(chain_rlp_path: &str) -> Vec<Block> {
    let chain_file = std::fs::File::open(chain_rlp_path).expect("Failed to open chain rlp file");
    decode::chain_file(chain_file).expect("Failed to decode chain rlp file")
}

pub fn parse_sync_mode(s: &str) -> eyre::Result<SyncMode> {
    match s {
        "full" => Ok(SyncMode::Full),
        "snap" => Ok(SyncMode::Snap),
        other => Err(eyre::eyre!(
            "Invalid syncmode {other:?} expected either snap or full",
        )),
    }
}

pub fn parse_socket_addr(addr: &str, port: &str) -> io::Result<SocketAddr> {
    // NOTE: this blocks until hostname can be resolved
    format!("{addr}:{port}")
        .to_socket_addrs()?
        .next()
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Failed to parse socket address",
        ))
}

pub fn default_datadir() -> PathBuf {
    let app_name: &'static str = "ethrex";
    let project_dir = ProjectDirs::from("", "", app_name).expect("Couldn't find home directory");
    project_dir.data_local_dir().to_path_buf()
}

/// Ensures that the provided data directory exists and is a directory.
///
/// # Panics
///
/// Panics if the path points to something different than a directory, or
/// if the directory cannot be created.
pub fn init_datadir(datadir: &Path) {
    if datadir.exists() {
        if !datadir.is_dir() {
            panic!("Datadir {datadir:?} exists but is not a directory");
        }
    } else {
        std::fs::create_dir_all(datadir).expect("Failed to create data directory");
    }
}

pub fn store_node_config_file(config: NodeConfigFile, file_path: PathBuf) {
    let json = match serde_json::to_string(&config) {
        Ok(json) => json,
        Err(e) => {
            error!("Could not store config in file: {e:?}");
            return;
        }
    };

    if let Err(e) = std::fs::write(file_path, json) {
        error!("Could not store config in file: {e:?}");
    };
}

pub fn read_node_config_file(datadir: &Path) -> Result<Option<NodeConfigFile>, String> {
    const NODE_CONFIG_FILENAME: &str = "node_config.json";
    let file_path = datadir.join(NODE_CONFIG_FILENAME);
    if file_path.exists() {
        Ok(match std::fs::File::open(file_path) {
            Ok(file) => Some(
                serde_json::from_reader(file)
                    .map_err(|e| format!("Invalid node config file {e}"))?,
            ),
            Err(e) => return Err(format!("No config file found: {e}")),
        })
    } else {
        Ok(None)
    }
}

pub fn parse_private_key(s: &str) -> eyre::Result<SecretKey> {
    Ok(SecretKey::from_slice(&parse_hex(s)?)?)
}

pub fn parse_public_key(s: &str) -> eyre::Result<PublicKey> {
    Ok(PublicKey::from_slice(&parse_hex(s)?)?)
}

pub fn parse_hex(s: &str) -> eyre::Result<Bytes, FromHexError> {
    match s.strip_prefix("0x") {
        Some(s) => hex::decode(s).map(Into::into),
        None => hex::decode(s).map(Into::into),
    }
}

/// Returns a detailed client version struct with git info.
pub fn get_client_version() -> ethrex_rpc::ClientVersion {
    ethrex_rpc::ClientVersion::new(
        env!("CARGO_PKG_NAME").to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
        env!("VERGEN_GIT_BRANCH").to_string(),
        env!("VERGEN_GIT_SHA").to_string(),
        env!("VERGEN_RUSTC_HOST_TRIPLE").to_string(),
        env!("VERGEN_RUSTC_SEMVER").to_string(),
    )
}

/// Returns a detailed client version string with git info (for clap attributes).
pub fn get_client_version_string() -> String {
    get_client_version().to_string()
}

/// Returns a minimal client version string without git info.
pub fn get_minimal_client_version() -> String {
    format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

pub fn display_chain_initialization(genesis: &Genesis) {
    const BANNER: &str = include_str!("banner.txt");
    info!("\n{BANNER}");
    let border = "═".repeat(70);

    info!("{border}");
    info!("NETWORK CONFIGURATION");
    info!("{border}");

    for line in genesis.config.display_config().lines() {
        info!("{line}");
    }

    info!("");
    let hash = genesis.get_block().hash();
    info!("Genesis Block Hash: {hash:x}");
    info!("{border}");
}

pub fn is_memory_datadir(datadir: &Path) -> bool {
    datadir.ends_with("memory")
}
