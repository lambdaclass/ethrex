//! Teardown contract for the RPC test context.
//!
//! Every `default_context_with_storage` context owns a `block_executor` OS thread
//! and a mempool prewarmer thread (plus that prewarmer's own rayon pool). Both hold
//! strong `Arc<Blockchain>` references, and a `Blockchain` owns a 17-thread
//! merkleization pool -- so a context that outlives its test keeps ~21 threads
//! alive.
//!
//! Those threads do exit on their own once the context drops, but *asynchronously*:
//! a full run of this binary creates them faster than the OS reaps them, so a
//! standing backlog builds up. Measured on this suite, the two tests below took
//! the peak from ~3250 live threads to ~2230 by removing ~1100 `merkle-worker`
//! threads. That backlog is what crosses the macOS CI runner's per-task thread cap
//! and aborts the whole binary with `libc++abi: terminating` / SIGABRT -- taking
//! libtest's failure report with it, so the logs show `FAILED` tests with no
//! explanation.
//!
//! Dropping the context must therefore be *synchronous*: once the drop returns, the
//! executor has been joined and the `Blockchain` released.

use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};
use std::sync::Arc;

/// Dropping the context must release the `Blockchain`. That is only observable once
/// every thread the context spawned has finished, because those threads hold the
/// only other strong references. A successful `Weak::upgrade` after the drop means a
/// thread -- and the 17-thread merkle pool it keeps alive -- outlived the test.
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

/// A context that only ever serves read-only RPC must not build the 17-thread
/// merkleization pool. Nothing on that path merkleizes, and nearly every test in
/// this binary is exactly this shape -- so building the pool per context is what
/// puts ~1100 `merkle-worker` threads into a full run.
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
        "serving read-only RPC built the 17-thread merkleization pool"
    );
}
