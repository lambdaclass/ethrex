use crate::decode;
use bytes::Bytes;
use directories::ProjectDirs;
use ethrex_blockchain::{add_block, fork_choice::apply_fork_choice};
use ethrex_common::types::{Block, Genesis};
use ethrex_p2p::{kademlia::KademliaTable, sync::SyncMode, types::Node};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::Store;
use ethrex_vm::{backends::EVM, EVM_BACKEND};
use std::{
    fs::File,
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

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

pub fn sync_mode(matches: &clap::ArgMatches) -> SyncMode {
    let syncmode = matches.get_one::<String>("syncmode");
    match syncmode {
        Some(mode) if mode == "full" => SyncMode::Full,
        Some(mode) if mode == "snap" => SyncMode::Snap,
        other => panic!("Invalid syncmode {:?} expected either snap or full", other),
    }
}

pub fn set_datadir(datadir: &str) -> String {
    let project_dir = ProjectDirs::from("", "", datadir).expect("Couldn't find home directory");
    project_dir
        .data_local_dir()
        .to_str()
        .expect("invalid data directory")
        .to_owned()
}

pub fn import_blocks(store: &Store, blocks: &Vec<Block>) {
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

pub fn read_known_peers(file_path: PathBuf) -> Result<Vec<Node>, serde_json::Error> {
    let Ok(file) = std::fs::File::open(file_path) else {
        return Ok(vec![]);
    };

    serde_json::from_reader(file)
}
