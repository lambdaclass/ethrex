use clap::Parser;
use ethrex::networks::Network;
use ethrex::{
    initializers::{get_local_node_record, get_signer, init_blockchain, init_store},
    utils::get_client_version,
};
use ethrex_blockchain::BlockchainType;
use ethrex_common::types::Genesis;
use ethrex_common::{Bytes, types::ChainConfig};
use ethrex_common::{H256, H512};
use ethrex_p2p::kademlia;
use ethrex_p2p::utils::public_key_from_signing_key;
use ethrex_p2p::{
    kademlia::Kademlia,
    rlpx::{
        connection::server::{CastMessage, RLPxConnection},
        message::Message,
        snap::GetTrieNodes,
    },
};
use ethrex_p2p::{network::P2PContext, peer_handler::MAX_RESPONSE_BYTES};
use ethrex_p2p::{rlpx::snap::TrieNodes, types::Node};
use ethrex_storage::{EngineType, Store, error::StoreError};
use ethrex_trie::{Nibbles, Node as TrieNode};
use ethrex_vm::EvmEngine;
use serde::{Deserialize, Serialize};
use spawned_concurrency::error::GenServerError;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::{net::Ipv4Addr, str::FromStr};
use std::{net::Ipv6Addr, time::Duration};
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info, metadata::Level};
use tracing_subscriber::{EnvFilter, FmtSubscriber, filter::Directive};

pub fn init_tracing(log_level: Level) {
    let log_filter = EnvFilter::builder()
        .with_default_directive(Directive::from(log_level))
        .from_env_lossy();
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(log_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

/// Simple function that creates a p2p context with a bunch of default values
/// we assume ports, levm and inmemory implementation
async fn get_p2p_context(network: String) -> Result<P2PContext, StoreError> {
    let genesis: Genesis = match network.as_str() {
        "localnet" => {
            let file =
                File::open("local_testnet_data/genesis.json").expect("Failed to open genesis file");
            let reader = BufReader::new(file);
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file")
        }
        "hoodi" => Network::from("hoodi")
            .get_genesis()
            .expect("We should have the genesis hoodi"),
        &_ => Network::mainnet()
            .get_genesis()
            .expect("We should have the genesis mainnet"),
    };
    let data_dir = "jwt/secrets";
    let signer = get_signer(data_dir);
    let local_node = Node::new(
        Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0).into(),
        30303, // Check this number, doesn't matter for now,
        30303,
        public_key_from_signing_key(&signer),
    );
    let local_node_record = Arc::new(Mutex::new(get_local_node_record(
        data_dir,
        &local_node,
        &signer,
    )));
    let tracker = TaskTracker::new();
    let peer_table = Kademlia::new();
    let storage = init_store("memory", genesis).await;
    let blockchain = init_blockchain(EvmEngine::LEVM, storage.clone(), BlockchainType::L1);
    Ok(P2PContext::new(
        local_node,
        local_node_record,
        tracker,
        signer,
        peer_table,
        storage,
        blockchain,
        get_client_version(),
        None,
    ))
}

const SAI_TEST_TOKEN: &'static str =
    "fff2bef58e73f6f4be26bef5ede84778aa946b1e2253e1943d4acdf1d7c384e7";

const UNISWAP_TEST_TOKEN: &'static str =
    "22002fe30a172d0a479f6add89c63b29dce29b6071b3c7e486b0fb4bc431f885";

#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    #[error("DB error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Hex Decode error: {0}")]
    FromHexError(#[from] rustc_hex::FromHexError),
    #[error("Genserver error: {0}")]
    GenServerError(#[from] GenServerError),
}

pub fn read_config(file_path: String) -> Config {
    let file = File::open(file_path).expect("Failed to open file");
    let reader = BufReader::new(file);
    serde_json::from_reader(reader).expect("Failed to deserialize genesis file")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    node: Node,
    network: String,
}

#[derive(Parser, Debug)]
pub struct Options {
    #[arg(
        long = "config",
        value_name = "CONFIG_FILE_PATH",
        help = "Receives the path to a `Config` struct in json format"
    )]
    pub config: String,
    #[arg(help = "Receives the state root to ask the node")]
    pub state_root: H256,
    #[arg(
        help = "Receives the nibbles to be search in the account. The format is the bytes separated by commas. example: 123,abc",
        default_value = ""
    )]
    pub nibbles: String,
}

fn insert_zeros(input: &str) -> String {
    let mut output: Vec<char> = Vec::new();
    for character in input.chars() {
        output.push('0');
        output.push(character as char);
    }
    output.into_iter().collect()
}

fn parse_bytes(nibbles: String) -> Vec<Bytes> {
    if nibbles.is_empty() {
        return vec![Bytes::from(Nibbles::default().encode_compact())];
    }
    nibbles
        .split(',')
        .map(|input| insert_zeros(input))
        .map(|input| hex::decode(input).expect("should work"))
        .map(|hex| Nibbles::from_hex(hex))
        .map(|nibs| Bytes::from(nibs.encode_compact()))
        .collect()
}

fn print_trie_nodes(nodes: TrieNodes) {
    info!(
        "Printing trie nodes. We got {} nodes from the peers.",
        nodes.nodes.len()
    );
    for node_bytes in nodes.nodes {
        let node =
            TrieNode::decode_raw(&node_bytes).expect("The node shouldn't send byzantine data");
        match node {
            TrieNode::Branch(branch_node) => {
                info!(
                    "We got a branch node with the childrens {:?}",
                    branch_node
                        .choices
                        .iter()
                        .enumerate()
                        .filter_map(|(index, child)| {
                            if child.is_valid() { Some(index) } else { None }
                        })
                        .collect::<Vec<usize>>()
                )
            }
            TrieNode::Extension(extension_node) => info!("We got an extension node with the data"),
            TrieNode::Leaf(leaf_node) => {
                info!("We got a leaf node")
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), ConsoleError> {
    let opts = Options::parse();
    let Config { node, network } = read_config(opts.config);
    let trienodes = parse_bytes(opts.nibbles);
    println!("{:?}", trienodes);

    init_tracing(Level::DEBUG);

    let p2p_context = get_p2p_context(network).await?;

    let _ = RLPxConnection::spawn_as_initiator(p2p_context.clone(), &node).await;
    let mut peer_channel = p2p_context
        .table
        .get_peer_channel(node.node_id())
        .await
        .expect("We should have the node we have spawned");

    let sai_account = H256::from_str(SAI_TEST_TOKEN)?.0;
    let account_path = sai_account.to_vec();
    let uniswap_account = H256::from_str(UNISWAP_TEST_TOKEN)?.0;
    let uniswap_path = sai_account.to_vec();

    let mut paths = vec![vec![Bytes::from(account_path)]];

    paths[0].extend(trienodes.clone());

    let gtn = GetTrieNodes {
        id: 0,
        root_hash: opts.state_root,
        paths,
        bytes: MAX_RESPONSE_BYTES,
    };

    info!("Sending the gtn {gtn:?}");

    peer_channel
        .connection
        .cast(CastMessage::BackendMessage(Message::GetTrieNodes(gtn)))
        .await?;

    let mut receiver = peer_channel.receiver.lock().await;

    match receiver.recv().await {
        Some(Message::TrieNodes(nodes)) => {
            print_trie_nodes(nodes);
        }
        _ => error!("We received a random message"),
        None => error!("Connection closed unexpectedly"),
    }

    Ok(())
}
