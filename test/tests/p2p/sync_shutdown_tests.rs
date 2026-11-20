//! An irrecoverable sync error must shut the node down through its cancel token, not by
//! exiting the process. `main` then runs the shutdown sequence (database flush and the
//! known-peers dump to `node_config.json`) and still exits non-zero. A `process::exit`
//! from the sync task skipped all of it (#5636).

use std::{sync::Arc, time::Duration};

use ethrex_blockchain::Blockchain;
use ethrex_common::{H256, types::Genesis};
use ethrex_p2p::peer_table::PeerTableServerProtocol;
use ethrex_p2p::sync::{BackfillConfig, HistoryChain, SyncMode, fatal_sync_error};
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_rpc::test_utils::dummy_peer_handler;
use ethrex_storage::{EngineType, Store};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

#[tokio::test(flavor = "multi_thread")]
async fn irrecoverable_sync_error_cancels_the_node_instead_of_exiting() {
    let genesis_file = include_bytes!("../../../fixtures/genesis/l1.json");
    let genesis: Genesis = serde_json::from_slice(genesis_file).unwrap();
    let mut store = Store::new("", EngineType::InMemory).unwrap();
    store.add_initial_state(genesis).await.unwrap();
    let blockchain = Arc::new(Blockchain::default_with_store(store.clone()));

    // A stopped peer table makes the cycle's first peer request fail with
    // `ActorStopped`, which the sync treats as irrecoverable.
    let peer_handler = dummy_peer_handler(store.clone()).await;
    peer_handler.peer_table.shutdown().unwrap();

    let cancel_token = CancellationToken::new();
    let sync_manager = SyncManager::new(
        peer_handler,
        &SyncMode::Full,
        cancel_token.clone(),
        blockchain,
        store.clone(),
        ".".into(),
        BackfillConfig {
            mode: HistoryChain::Off,
            tx_index_horizon: 0,
        },
        TaskTracker::new(),
    )
    .await;

    // A cold node (only genesis) starts a sync cycle right away.
    sync_manager.sync_to_head(H256::random());

    // Before the fix the sync task called `process::exit(2)` here, which killed this
    // test binary instead of cancelling the token.
    tokio::time::timeout(Duration::from_secs(10), cancel_token.cancelled())
        .await
        .expect("an irrecoverable sync error must cancel the node token");
    assert!(
        fatal_sync_error().is_some(),
        "the sync error must be recorded so the node exits non-zero"
    );
}
