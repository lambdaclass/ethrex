//! Teardown contract for the RPC test context.
//!
//! A context used to keep ~21 threads alive per test: a `block_executor`, a mempool
//! prewarmer with its own rayon pool, and the merkleization pool that the
//! `Arc<Blockchain>` those two held kept alive with them. They exited on their own
//! but *asynchronously*, so a full run of this binary built a standing backlog --
//! and that backlog is what crosses the macOS CI runner's per-task thread cap and
//! aborts the whole binary with `libc++abi: terminating` / SIGABRT, taking
//! libtest's failure report with it, so the logs show `FAILED` tests with no
//! explanation.
//!
//! The two tests below pin the invariants of the *context*: it spawns no prewarmer,
//! and it builds no merkleization pool on a read-only path. Dropping it is
//! synchronous too -- the executor is joined before the drop returns. The
//! constructor-level invariants live in `tests/blockchain/merkle_pool_tests.rs`.
//!
//! What remains at peak is almost entirely `tokio-rt-worker` threads from the
//! per-test runtimes' blocking pools, so further headroom has to come from there.
use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};
use std::sync::Arc;

/// Dropping the context must release the `Blockchain`. That is only observable once
/// the `block_executor` thread has finished, because it holds the only other strong
/// reference. A successful `Weak::upgrade` after the drop means that thread -- and
/// any merkle pool it built -- outlived the test.
///
/// The limit of what this proves: rayon does not join a pool's workers on drop, so
/// the assertion is that nothing keeps the pool alive -- not that its threads are
/// already gone.
#[tokio::test]
async fn dropping_the_test_context_releases_its_blockchain() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let blockchain = Arc::downgrade(&context.blockchain);
    assert!(
        blockchain.upgrade().is_some(),
        "sanity check: the context should hold its blockchain while alive"
    );

    drop(context);

    assert!(
        blockchain.upgrade().is_none(),
        "a thread spawned by the context still holds the Blockchain after the \
         context was dropped: teardown is asynchronous, so the context's merkle \
         pool leaks into the rest of the run"
    );
}

/// A context that only ever serves read-only RPC must not build the merkleization
/// pool. Nothing on that path merkleizes, and nearly every test in this binary is
/// exactly this shape -- so a pool per context was the single largest contributor
/// to the `merkle-worker` backlog in a full run.
#[tokio::test]
async fn a_read_only_context_never_builds_the_merkle_pool() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = r#"{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}"#.to_string();
    let response = call_http(&context, body).await;
    assert!(
        response.get("result").is_some(),
        "eth_blockNumber should succeed, got: {response}"
    );

    assert!(
        !context.blockchain.merkle_pool_initialized(),
        "serving read-only RPC built the merkleization pool"
    );
}
