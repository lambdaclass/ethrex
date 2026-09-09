//! In-process engine-API RpcApiContext factory for the ef_tests-engine harness.
//!
//! Lives in the test tooling (not in `ethrex-rpc`) because the shared statics
//! below exist solely to amortise per-fixture cost across the ~5600 fixtures
//! this crate runs. Production has no reason to share a `SyncManager` across
//! `RpcApiContext`s. (The merkleization pool needs no sharing: `Blockchain`
//! builds it on first use, so a fixture that never merkleizes costs no threads.)

use std::sync::Arc;

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::DEFAULT_BUILDER_GAS_CEIL;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_rpc::{
    ClientVersion, GasTipEstimator, NodeData, RpcApiContext, start_block_executor,
    test_utils::{all_namespaces_for_tests, dummy_sync_manager, example_shared_local_node},
};
use ethrex_storage::Store;
use tokio::sync::{Mutex as TokioMutex, OnceCell};

/// Shared SyncManager for `engine_only_context`. Allocated once per process so the
/// RLPxInitiator OS thread (spawned by `dummy_actor::spawn_on_thread`) is created
/// exactly once regardless of how many harnesses are built.
///
/// Verified unused in the engine and eth/block handler paths:
///   `rg "context\.peer_handler|ctx\.peer_handler" crates/networking/rpc/engine/` -> empty
///   `rg "context\.peer_handler|ctx\.peer_handler" crates/networking/rpc/eth/block.rs` -> empty
static SHARED_SYNCER: OnceCell<Arc<SyncManager>> = OnceCell::const_new();

/// In-process engine-API context for testing, sharing the P2P scaffold across calls.
///
/// Reuses a single `Arc<SyncManager>` per process (via `SHARED_SYNCER`), so the
/// RLPxInitiator OS thread is allocated exactly once regardless of fixture count.
/// `peer_handler` is `None`; the engine handlers and `eth_getBlockByNumber` do not
/// touch it (confirmed by the `rg` invariants above).
/// `syncer` is `Some(shared)` with `SyncMode::Full`, satisfying the engine handler
/// requirements in `engine_forkchoiceUpdated*` and `engine_newPayload*`.
pub async fn engine_only_context(storage: Store) -> RpcApiContext {
    let shared_syncer = SHARED_SYNCER
        .get_or_init(|| async { Arc::new(dummy_sync_manager().await) })
        .await
        .clone();
    let blockchain = Arc::new(Blockchain::for_test_harness(storage.clone()));
    // The runner owns this context for its whole lifetime, so the executor thread
    // is left detached.
    let (block_worker_channel, _executor) = start_block_executor(blockchain.clone());
    RpcApiContext {
        storage,
        blockchain,
        active_filters: Default::default(),
        syncer: Some(shared_syncer),
        peer_handler: None,
        node_data: NodeData {
            jwt_secret: Default::default(),
            shared_local_node: example_shared_local_node(),
            client_version: ClientVersion::new(
                "ethrex".to_string(),
                "0.1.0".to_string(),
                "test".to_string(),
                "abcd1234".to_string(),
                "x86_64-unknown-linux".to_string(),
                "1.70.0".to_string(),
            ),
            extra_data: Bytes::new(),
        },
        gas_tip_estimator: Arc::new(TokioMutex::new(GasTipEstimator::new())),
        log_filter_handler: None,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        block_worker_channel,
        ws: None,
        allowed_namespaces: Arc::new(all_namespaces_for_tests()),
    }
}
