use ethrex_p2p::{kademlia::Kademlia, rlpx::connection::server::RLPxConnection};
use std::net::Ipv6Addr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;

use ethrex::{
    initializers::{get_local_node_record, get_signer, init_blockchain},
    utils::get_client_version,
};
use ethrex_blockchain::BlockchainType;
use ethrex_common::H512;
use ethrex_p2p::network::P2PContext;
use ethrex_p2p::types::Node;
use ethrex_p2p::utils::public_key_from_signing_key;
use ethrex_storage::{EngineType, Store, error::StoreError};
use ethrex_vm::EvmEngine;

/// Simple function that creates a p2p context with a bunch of default values
/// we assume ports, levm and inmemory implementation
fn get_p2p_context() -> Result<P2PContext, StoreError> {
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
    let storage = Store::new("", EngineType::InMemory)?;
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

const SAI_TEST_TOKEN: &'static str = "0x89d24A6b4CcB1B6fAA2625fE562bDD9a23260359";

const OTHER_NODE_PUBLIC_KEY: &'static str = "f070a8dc0ec1b1ca687e9e26cd57a70fca2957c37f801ace47f9cc9d7e50e8267a3972653b2dc4dc4b02b269017db4b1f2fd29231d1d275f5fc2397ca05774d3";

#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    #[error("DB error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Hex Decode error: {0}")]
    FromHexError(#[from] rustc_hex::FromHexError),
}

#[tokio::main]
async fn main() -> Result<(), ConsoleError> {
    let p2p_context = get_p2p_context()?;
    let other_node = Node::new(
        Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0).into(),
        30303, // Check this number, doesn't matter for now,
        30303,
        H512::from_str(OTHER_NODE_PUBLIC_KEY)?,
    );

    let rlpx_connection = RLPxConnection::spawn_as_initiator(p2p_context, &other_node).await;

    Ok(())
}
