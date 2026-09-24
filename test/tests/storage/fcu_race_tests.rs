//! Reproducer for the TOCTOU race in `Store::forkchoice_update_inner`.
//!
//! The inner function reads `LatestBlockNumber` from the DB before entering the
//! write transaction, then uses that captured value to compute the delete range
//! `(head+1..=latest)` and to unconditionally write `LatestBlockNumber`. Two
//! concurrent callers can each capture a stale `latest` and leave the canonical
//! table with entries above the persisted `LatestBlockNumber`.
//!
//! Per-iteration probe:
//!   1. seed canonical 0..=BASE with latest=BASE.
//!   2. spawn two concurrent FCUs:
//!      A: extension to head=BASE+EXT with new_canonical=[BASE+1..=BASE+EXT].
//!      B: trivial FCU at head=BASE (empty new_canonical, empty delete range).
//!   3. after both complete, call a cleanup FCU(head=BASE).
//!      - if DB latest == BASE+EXT (A's commit won), cleanup deletes BASE+1..=BASE+EXT.
//!      - if DB latest == BASE (B's commit overwrote with a stale view of latest),
//!        cleanup's delete range is empty and A's canonical entries remain as orphans.
//!   4. any canonical entry in BASE+1..=BASE+EXT after cleanup => race hit.
//!
//! BASE and EXT are chosen so the extension spans block 256, crossing the
//! boundary where `u64::to_le_bytes()` stops fitting in a single byte. This
//! guards against future refactors that try to replace the delete loop with a
//! byte-range deletion — LE-encoded keys are not lexicographically monotone,
//! so a range-delete would silently leave some orphans behind.
//!
//! Threshold calibration (pre-fix, multi-thread tokio runtime, N iterations):
//!   - N=5: 4999/5000 trials hit the race.
//!   - N=10: 5000/5000.
//!
//! The test loop runs 100 iterations (~20 ms).

use bytes::Bytes;
use ethrex_common::{H256, types::BlockHeader};
use ethrex_storage::{EngineType, Store};

const BASE: u64 = 250;
const EXT: u64 = 10;
const HEADER_COUNT: u64 = BASE + EXT + 2;
const ITERATIONS: usize = 100;

fn build_headers() -> (Vec<BlockHeader>, Vec<H256>) {
    let mut headers = Vec::with_capacity(HEADER_COUNT as usize);
    let mut hashes = Vec::with_capacity(HEADER_COUNT as usize);
    let mut parent_hash = H256::zero();
    for n in 0..HEADER_COUNT {
        let h = BlockHeader {
            parent_hash,
            number: n,
            extra_data: Bytes::from(n.to_le_bytes().to_vec()),
            ..Default::default()
        };
        let hash = h.hash();
        parent_hash = hash;
        hashes.push(hash);
        headers.push(h);
    }
    (headers, hashes)
}

async fn race_iteration(store: &Store, hashes: &[H256]) -> bool {
    let seed: Vec<_> = (0..=BASE).map(|n| (n, hashes[n as usize])).collect();
    store
        .forkchoice_update(seed, BASE, hashes[BASE as usize], None, None)
        .await
        .expect("seed FCU");

    let s_a = store.clone();
    let s_b = store.clone();
    let ext_canonical: Vec<_> = (BASE + 1..=BASE + EXT)
        .map(|n| (n, hashes[n as usize]))
        .collect();
    let base_hash = hashes[BASE as usize];
    let ext_head_hash = hashes[(BASE + EXT) as usize];

    let ta = tokio::spawn(async move {
        s_a.forkchoice_update(ext_canonical, BASE + EXT, ext_head_hash, None, None)
            .await
    });
    let tb = tokio::spawn(async move {
        s_b.forkchoice_update(vec![], BASE, base_hash, None, None)
            .await
    });
    // Surface task panics / FCU errors: a silently-failed task could mask the
    // race by skipping its side of the interleave.
    let (ra, rb) = tokio::join!(ta, tb);
    ra.expect("task A panicked").expect("FCU A failed");
    rb.expect("task B panicked").expect("FCU B failed");

    store
        .forkchoice_update(vec![], BASE, base_hash, None, None)
        .await
        .expect("cleanup FCU");

    for n in BASE + 1..=BASE + EXT {
        if store
            .get_canonical_block_hash_sync(n)
            .expect("read canonical")
            .is_some()
        {
            return true;
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forkchoice_update_is_concurrency_safe() {
    let store = Store::new("", EngineType::InMemory).expect("build store");
    let (headers, hashes) = build_headers();
    store
        .add_block_headers(headers)
        .await
        .expect("seed headers");

    for iter in 0..ITERATIONS {
        if race_iteration(&store, &hashes).await {
            panic!(
                "forkchoice_update race detected at iteration {iter}: \
                 canonical entry exists above LatestBlockNumber after cleanup"
            );
        }
    }
}
