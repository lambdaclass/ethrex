use ethrex::networks::Network;
use ethrex::{
    initializers::{get_local_node_record, get_signer, init_blockchain, init_store},
    utils::get_client_version,
};
use ethrex_blockchain::BlockchainType;
use ethrex_common::types::Genesis;
use ethrex_common::{Bytes, types::ChainConfig};
use ethrex_common::{H256, H512};
use ethrex_p2p::rlpx::connection::server::RLPxReceiver;
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
use ethrex_trie::Nibbles;
use ethrex_vm::EvmEngine;
use spawned_concurrency::error::GenServerError;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::{net::Ipv4Addr, str::FromStr};
use std::{net::Ipv6Addr, time::Duration};
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::{error, info, metadata::Level};
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
async fn get_p2p_context() -> Result<P2PContext, StoreError> {
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
    let file = File::open("/Users/mateo/ethrex/local_testnet_data/genesis.json")
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
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
    "998abd7945acf1765167f39605e218cbad5644f90c6fa434177865c14c218cf2";

const OTHER_NODE_PUBLIC_KEY: &'static str = "dbfa18978b2de8b8e7dddaebc27f0b9d9e4a021bb3a9130eded115f96cfa8b8d20baeb7e12338810995f3bed89d0c1438da1292af3de4434818629cad49f1165";

#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    #[error("DB error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Hex Decode error: {0}")]
    FromHexError(#[from] rustc_hex::FromHexError),
    #[error("Genserver error: {0}")]
    GenServerError(#[from] GenServerError),
}

#[tokio::main]
async fn main() -> Result<(), ConsoleError> {
    init_tracing(Level::DEBUG);

    let p2p_context = get_p2p_context().await?;
    let other_node = Node::new(
        Ipv4Addr::new(127, 0, 0, 1).into(),
        51800, // Check this number, doesn't matter for now,
        51800,
        H512::from_str(OTHER_NODE_PUBLIC_KEY)?,
    );

    let _ = p2p_context.set_fork_id().await;
    let mut rlpx_connection = RLPxConnection::spawn_as_initiator(p2p_context, &other_node).await;
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<TrieNodes>(1000);

    tokio::time::sleep(Duration::from_secs(5)).await;

    let sai_account = H256::from_str(SAI_TEST_TOKEN)?.0;
    let account_path = sai_account.to_vec();

    let gtn = GetTrieNodes {
        id: 0,
        root_hash: H256::from_str(
            "a9cfa5bc8b546e8bccb850328ddb674f6a514d9f259ad94df833f3a0430f7b42",
        )?,
        paths: vec![vec![
            Bytes::from(account_path),
            Bytes::from(Nibbles::default().encode_compact()),
        ]],
        bytes: MAX_RESPONSE_BYTES,
    };

    info!("Sending the gtn {gtn:?}");

    rlpx_connection
        .cast(CastMessage::BackendRequest(
            Message::GetTrieNodes(gtn),
            RLPxReceiver::Channel(sender),
        ))
        .await?;

    match receiver.recv().await {
        Some(nodes) => info!("We got these trienodes {:?}", nodes),
        None => error!("Connection closed unexpectedly"),
    }

    Ok(())
}
