use ethrex_blockchain::Blockchain;
use ethrex_p2p::sync::SyncMode;
use ethrex_p2p::{
    discv4::peer_table::{PeerTable, TARGET_PEERS},
    network::P2PContext,
    peer_handler::PeerHandler,
    rlpx::initiator::RLPxInitiator,
    sync_manager::SyncManager,
    types::Node,
};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use spawned_concurrency::tasks::{GenServer, GenServerHandle};
use std::sync::Arc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::info;

/// Creates a dummy SyncManager for tests where syncing is not needed
/// This should only be used in tests as it won't be able to connect to the p2p network
pub async fn dummy_sync_manager() -> SyncManager {
    SyncManager::new(
        dummy_peer_handler().await,
        &SyncMode::Full,
        CancellationToken::new(),
        Arc::new(Blockchain::default_with_store(
            Store::new("", EngineType::InMemory).expect("Failed to start Store Engine"),
        )),
        Store::new("temp.db", ethrex_storage::EngineType::InMemory)
            .expect("Failed to start Storage Engine"),
        ".".into(),
    )
    .await
}

/// Creates a dummy PeerHandler for tests where interacting with peers is not needed
/// This should only be used in tests as it won't be able to interact with the node's connected peers
pub async fn dummy_peer_handler() -> PeerHandler {
    let peer_table = PeerTable::spawn(TARGET_PEERS);
    PeerHandler::new(peer_table.clone(), dummy_gen_server(peer_table).await)
}

/// Creates a dummy GenServer for tests
/// This should only be used in tests
pub async fn dummy_gen_server(peer_table: PeerTable) -> GenServerHandle<RLPxInitiator> {
    info!("Starting RLPx Initiator");
    let state = RLPxInitiator::new(dummy_p2p_context(peer_table).await);
    RLPxInitiator::start_on_thread(state)
}

/// Creates a dummy P2PContext for tests
/// This should only be used in tests as it won't be able to connect to the p2p network
pub async fn dummy_p2p_context(peer_table: PeerTable) -> P2PContext {
    let local_node = Node::from_enode_url(
        "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
    ).expect("Bad enode url");
    let storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");

    P2PContext::new(
        local_node,
        TaskTracker::default(),
        SecretKey::from_byte_array(&[0xcd; 32]).expect("32 bytes, within curve order"),
        peer_table,
        storage.clone(),
        Arc::new(Blockchain::default_with_store(storage)),
        "".to_string(),
        None,
        1000,
    )
    .await
    .unwrap()
}
