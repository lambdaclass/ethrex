use crate::decode;
use bytes::Bytes;
use directories::ProjectDirs;
use ethrex_common::types::{Block, Genesis};
use ethrex_p2p::{kademlia::KademliaTable, sync::SyncMode, types::Node};
use ethrex_rlp::decode::RLPDecode;
use ethrex_vm::backends::EvmEngine;
use hex::FromHexError;
#[cfg(feature = "l2")]
use secp256k1::SecretKey;
use std::{
    fs::File,
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::Mutex;
use tracing::{error, info};

pub fn read_jwtsecret_file(jwt_secret_path: &str) -> Bytes {
    match File::open(jwt_secret_path) {
        Ok(mut file) => decode::jwtsecret_file(&mut file),
        Err(_) => write_jwtsecret_file(jwt_secret_path),
    }
}

pub fn write_jwtsecret_file(jwt_secret_path: &str) -> Bytes {
    info!("JWT secret not found in the provided path, generating JWT secret");
    let secret = generate_jwt_secret();
    std::fs::write(jwt_secret_path, &secret).expect("Unable to write JWT secret file");
    hex::decode(secret).unwrap().into()
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

pub fn read_block_file(block_file_path: &str) -> Block {
    let encoded_block = std::fs::read(block_file_path)
        .unwrap_or_else(|_| panic!("Failed to read block file with path {}", block_file_path));
    Block::decode(&encoded_block)
        .unwrap_or_else(|_| panic!("Failed to decode block file {}", block_file_path))
}

pub fn read_genesis_file(genesis_file_path: &str) -> Genesis {
    let genesis_file = std::fs::File::open(genesis_file_path).expect("Failed to open genesis file");
    decode::genesis_file(genesis_file).expect("Failed to decode genesis file")
}

pub fn parse_evm_engine(s: &str) -> eyre::Result<EvmEngine> {
    EvmEngine::try_from(s.to_owned()).map_err(|e| eyre::eyre!("{e}"))
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

pub fn set_datadir(datadir: &str) -> String {
    let project_dir = ProjectDirs::from("", "", datadir).expect("Couldn't find home directory");
    project_dir
        .data_local_dir()
        .to_str()
        .expect("invalid data directory")
        .to_owned()
}

pub async fn store_known_peers(table: Arc<Mutex<KademliaTable>>, file_path: PathBuf) {
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

#[allow(dead_code)]
pub fn read_known_peers(file_path: PathBuf) -> Result<Vec<Node>, serde_json::Error> {
    let Ok(file) = std::fs::File::open(file_path) else {
        return Ok(vec![]);
    };

    serde_json::from_reader(file)
}

#[cfg(feature = "l2")]
pub fn parse_private_key(s: &str) -> eyre::Result<SecretKey> {
    Ok(SecretKey::from_slice(&parse_hex(s)?)?)
}

pub fn parse_hex(s: &str) -> eyre::Result<Bytes, FromHexError> {
    match s.strip_prefix("0x") {
        Some(s) => hex::decode(s).map(Into::into),
        None => hex::decode(s).map(Into::into),
    }
}
