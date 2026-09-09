//! Teardown contract for the RPC test context.
//!
//! A `default_context_with_storage` context used to own three kinds of thread: a
//! `block_executor`, a mempool prewarmer (plus that prewarmer's own rayon pool),
//! and — through the `Arc<Blockchain>` both of them held — a 17-thread
//! merkleization pool built eagerly in `Blockchain::new`. That is ~21 threads per
//! context, and they exited on their own but *asynchronously*: a full run of this
//! binary created them faster than the OS reaped them, so a standing backlog built
//! up. It is that backlog which crosses the macOS CI runner's per-task thread cap
//! and aborts the whole binary with `libc++abi: terminating` / SIGABRT -- taking
//! libtest's failure report with it, so the logs show `FAILED` tests with no
//! explanation.
//!
//! Three changes removed it. The two tests below pin the two that are invariants
//! of the *context*; the constructor-level ones are pinned by
//! `tests/blockchain/merkle_pool_tests.rs`:
//!
//! 1. `Blockchain::for_test_harness` disables the prewarmer, so no context spawns
//!    one -- nothing here exercises prewarming.
//! 2. The merkleization pool is built on first use, so a context that only serves
//!    read-only RPC never pays for it. (A harness where every instance *does*
//!    merkleize shares one pool instead; see `Blockchain::for_test_harness_with_pool`.)
//! 3. Dropping the context is synchronous: once the drop returns, the executor has
//!    been joined and the `Blockchain` released.
//!
//! Peak live threads for the whole `ethrex_tests` binary now measure ~2300 on a
//! 14-core macOS host, and a snapshot taken at that peak is almost entirely
//! `tokio-rt-worker` threads from the per-test runtimes' blocking pools: 954 of
//! them, against 2 `block_executor` and zero `merkle-worker`. So the remaining
//! headroom has to come from those runtimes, not from anything here.

use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};
use std::sync::Arc;

/// Dropping the context must release the `Blockchain`. That is only observable once
/// the `block_executor` thread has finished, because it holds the only other strong
/// reference. A successful `Weak::upgrade` after the drop means that thread -- and
/// any merkle pool it built -- outlived the test.
///
/// Note the limit of what this proves: releasing the `Arc` drops the pool, but
/// `rayon::ThreadPool`'s `Drop` only signals its 17 workers to terminate and does
/// not join them, so a context that *did* merkleize still leaves them winding down.
/// The assertion is that nothing keeps the pool alive, not that its threads are
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

/// A context that only ever serves read-only RPC must not build the 17-thread
/// merkleization pool. Nothing on that path merkleizes, and nearly every test in
/// this binary is exactly this shape -- so a pool per context was the single
/// largest contributor to the `merkle-worker` backlog in a full run.
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
