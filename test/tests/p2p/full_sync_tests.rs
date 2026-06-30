//! Full-sync resume-point tests.
//!
//! Regression coverage for the full-sync state-gap wedge (glamsterdam-devnet-5 issue #6):
//! an FCU can canonicalize a head before its state is computed (`apply_fork_choice` gates
//! only on the branch link block), leaving canonical-but-stateless blocks. Full sync must
//! re-execute those rather than treat them as already-synced; skipping them anchored
//! execution on a parent with no state and wedged the node forever on `state root missing`.
//!
//! These tests exercise `is_resume_point`, the predicate the full-sync walk-back uses to
//! decide where to resume execution. A block is a valid resume point only if it is
//! canonical AND its post-state is present on disk.

use ethrex_common::{H256, types::BlockHeader};
use ethrex_p2p::sync::{first_resume_point_in_batch, is_resume_point};
use ethrex_storage::{EngineType, Store};
use ethrex_trie::EMPTY_TRIE_HASH;

/// Header at `number` with `state_root`, chained off `parent`.
fn header(number: u64, state_root: H256, parent: H256) -> BlockHeader {
    BlockHeader {
        number,
        state_root,
        parent_hash: parent,
        ..Default::default()
    }
}

/// In-memory store seeded with `headers`, marking the `canonical` ones canonical via FCU.
/// No EVM state is written: `has_state_root` returns true for `EMPTY_TRIE_HASH` and false
/// for any other (absent) root, which is exactly the present/absent distinction we need.
async fn seed_store(headers: &[BlockHeader], canonical: &[&BlockHeader]) -> Store {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    store
        .add_block_headers(headers.to_vec())
        .await
        .expect("add headers");
    if let Some(head) = canonical.last() {
        let list: Vec<(u64, H256)> = canonical.iter().map(|h| (h.number, h.hash())).collect();
        store
            .forkchoice_update(list, head.number, head.hash(), None, None)
            .await
            .expect("forkchoice update");
    }
    store
}

#[tokio::test]
async fn canonical_and_stateful_is_resume_point() {
    let h = header(1, *EMPTY_TRIE_HASH, H256::zero());
    let store = seed_store(std::slice::from_ref(&h), &[&h]).await;
    assert!(
        is_resume_point(&store, &h).unwrap(),
        "canonical block with present state must be a resume point"
    );
}

#[tokio::test]
async fn canonical_but_stateless_is_not_resume_point() {
    // Canonical, but post-state absent: the gap the fix must re-execute, not skip.
    let h = header(1, H256::from_low_u64_be(0xdead), H256::zero());
    let store = seed_store(std::slice::from_ref(&h), &[&h]).await;
    assert!(
        !is_resume_point(&store, &h).unwrap(),
        "canonical-but-stateless block must NOT be a resume point (else full sync wedges)"
    );
}

#[tokio::test]
async fn non_canonical_is_not_resume_point() {
    // Present state but never canonicalized -> not a resume point.
    let h = header(1, *EMPTY_TRIE_HASH, H256::zero());
    let store = seed_store(std::slice::from_ref(&h), &[]).await;
    assert!(!is_resume_point(&store, &h).unwrap());
}

#[tokio::test]
async fn walk_back_anchors_on_state_head_not_canonical_head() {
    // Blocks 1..=5 canonical; state present only up to block 2 (the state head). Blocks
    // 3..=5 are canonical-but-stateless (the gap); 6..=7 are the non-canonical new blocks.
    let mut chain = Vec::new();
    let mut parent = H256::zero();
    for number in 1..=7u64 {
        let state_root = if number <= 2 {
            *EMPTY_TRIE_HASH
        } else {
            H256::from_low_u64_be(0xc0de_0000 + number)
        };
        let h = header(number, state_root, parent);
        parent = h.hash();
        chain.push(h);
    }
    let canonical: Vec<&BlockHeader> = chain[0..5].iter().collect(); // 1..=5
    let store = seed_store(&chain, &canonical).await;

    // Full sync scans headers newest -> oldest looking for the first resume point.
    let newest_to_oldest: Vec<BlockHeader> = chain.iter().rev().cloned().collect();
    let first_resumable = newest_to_oldest
        .iter()
        .position(|h| is_resume_point(&store, h).unwrap());

    // First resume point is block 2 (the state head), at index 5 (after 7,6,5,4,3). So
    // blocks 7,6 (new) and 5,4,3 (the canonical-but-stateless gap) are all kept and
    // re-executed. The pre-fix logic stopped at the first *canonical* block (5) and would
    // have skipped the gap, wedging execution on block 3's missing parent state.
    assert_eq!(first_resumable, Some(5));
    assert_eq!(newest_to_oldest[5].number, 2);
}

/// Builds blocks 1..=`tip`, all canonical, with present state only on `stateful` block numbers.
/// Returns (store, full chain). Mirrors a pruned layered store: the executed/state head plus a
/// recent window have state; older canonical blocks do not.
async fn seed_chain(tip: u64, stateful: &[u64]) -> (Store, Vec<BlockHeader>) {
    let mut chain = Vec::new();
    let mut parent = H256::zero();
    for number in 1..=tip {
        let state_root = if stateful.contains(&number) {
            *EMPTY_TRIE_HASH
        } else {
            H256::from_low_u64_be(0xc0de_0000 + number)
        };
        let h = header(number, state_root, parent);
        parent = h.hash();
        chain.push(h);
    }
    let canonical: Vec<&BlockHeader> = chain.iter().collect();
    let store = seed_store(&chain, &canonical).await;
    (store, chain)
}

/// Regression for the issue #9 overshoot (introduced by tightening the walk-back merge from
/// `is_canonical_sync` to `is_resume_point`). The node's stateful head sits in the MIDDLE of a
/// batch whose oldest block is canonical-but-pruned. The walk must merge at the in-batch resume
/// point, not blow past it because the batch's bottom edge isn't a resume point.
#[tokio::test]
async fn walk_back_merges_at_resume_point_inside_batch() {
    // Canonical 1..=8, head = 8 = state head (only block 8 has state; 1..=7 are pruned).
    // New blocks 9,10 are not yet canonical. local_head = 8.
    let (store, chain) = seed_chain(8, &[8]).await;
    let local_head = 8;
    let new_blocks = [
        header(10, H256::from_low_u64_be(0xaaaa), chain[7].hash()), // 10 (parent 9 placeholder)
        header(9, H256::from_low_u64_be(0xbbbb), chain[7].hash()),  // 9
    ];

    // Batch newest->oldest spanning the new blocks down past the state head: [10, 9, 8, 7, 6, 5].
    let batch: Vec<BlockHeader> = new_blocks
        .iter()
        .cloned()
        .chain(chain[4..8].iter().rev().cloned()) // 8,7,6,5
        .collect();

    // The in-batch scan finds block 8 (canonical+stateful) at index 2, so the walk merges there
    // and executes 10,9 — instead of descending to genesis.
    let idx = first_resume_point_in_batch(&store, &batch, local_head)
        .unwrap()
        .expect("must find the in-batch resume point");
    assert_eq!(batch[idx].number, 8);

    // The pre-fix break checked only the parent of the batch's OLDEST header (block 4), which is
    // canonical-but-pruned -> not a resume point -> old logic overshot. Document that.
    assert!(!is_resume_point(&store, &chain[3]).unwrap());
}

/// A batch entirely above `local_head` cannot contain a resume point; the scan is skipped
/// (returns None) so the walk keeps descending without per-header state lookups.
#[tokio::test]
async fn batch_entirely_above_local_head_is_skipped() {
    let (store, chain) = seed_chain(8, &[8]).await;
    let new_blocks = vec![
        header(10, H256::from_low_u64_be(0xaaaa), chain[7].hash()),
        header(9, H256::from_low_u64_be(0xbbbb), chain[7].hash()),
    ];
    assert_eq!(
        first_resume_point_in_batch(&store, &new_blocks, 8).unwrap(),
        None
    );
}

/// A batch below the head but entirely pruned (no state) yields no resume point; the walk must
/// continue (and ultimately hit the pruned-base guard), not falsely merge.
#[tokio::test]
async fn batch_with_no_retained_state_returns_none() {
    // Canonical 1..=8 but NO state anywhere (fully pruned). local_head = 8.
    let (store, chain) = seed_chain(8, &[]).await;
    let batch: Vec<BlockHeader> = chain[4..8].iter().rev().cloned().collect(); // 8,7,6,5
    assert_eq!(
        first_resume_point_in_batch(&store, &batch, 8).unwrap(),
        None
    );
}
