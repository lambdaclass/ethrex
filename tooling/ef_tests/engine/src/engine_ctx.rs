//! In-process engine-API RpcApiContext factory for the ef_tests-engine harness.
//!
//! Lives in the test tooling (not in `ethrex-rpc`) because the shared statics
//! and the thread-local rayon pool below exist solely to amortise per-fixture
//! cost across the ~5600 fixtures this crate runs. Production has no reason
//! to share a `SyncManager` across `RpcApiContext`s or to hand out a single
//! merkle pool per worker thread.

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

thread_local! {
    /// Per-OS-thread merkleization pool, lazily built on first use.
    /// The merkle protocol requires its 16 worker jobs to run concurrently and
    /// communicate via channels, so each pool can have only ONE concurrent
    /// `in_place_scope` caller. Keying by `thread_local!` makes the calling
    /// tokio worker thread the natural exclusive owner of its pool — there are
    /// at most `num_cpus` worker threads alive, so total OS-thread cost is
    /// bounded by `num_cpus * 17` instead of `fixture_count * 17`.
    static THREAD_LOCAL_MERKLE_POOL: std::cell::OnceCell<Arc<rayon::ThreadPool>> =
        const { std::cell::OnceCell::new() };
}

fn thread_local_merkle_pool() -> Arc<rayon::ThreadPool> {
    THREAD_LOCAL_MERKLE_POOL.with(|cell| cell.get_or_init(Blockchain::build_merkle_pool).clone())
}

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
    let blockchain = Arc::new(Blockchain::default_with_store_and_pool(
        storage.clone(),
        thread_local_merkle_pool(),
    ));
    let block_worker_channel = start_block_executor(blockchain.clone());
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
