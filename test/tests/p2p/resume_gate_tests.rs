//! Resume-gate tests for `is_resume_point`: a block is a valid full-sync resume point
//! only if it is canonical AND its post-state is present on disk.

use ethrex_common::types::Genesis;
use ethrex_common::{H256, types::BlockHeader};
use ethrex_p2p::sync::is_resume_point;
use ethrex_storage::{EngineType, Store};
use ethrex_trie::EMPTY_TRIE_HASH;

/// Genesis is canonical and its state (EMPTY_TRIE_HASH) is always present, so it is a resume point.
#[tokio::test]
async fn is_resume_point_genesis_is_true() {
    let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(Genesis::default())
        .await
        .expect("seed genesis");

    let genesis_header = Genesis::default().get_block().header;
    let result = is_resume_point(&store, &genesis_header).expect("is_resume_point");
    assert!(result, "genesis must be a valid resume point");
}

/// A canonical header whose state is absent (state_root = H256::zero(), not on disk or in
/// any layer) must NOT be a resume point, else full sync would wedge re-executing on it.
#[tokio::test]
async fn is_resume_point_canonical_stateless_is_false() {
    let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(Genesis::default())
        .await
        .expect("seed genesis");

    let genesis_hash = Genesis::default().get_block().hash();
    let block1_header = BlockHeader {
        number: 1,
        parent_hash: genesis_hash,
        state_root: H256::zero(),
        ..Default::default()
    };
    let block1_hash = block1_header.hash();

    store
        .add_block_headers(vec![block1_header.clone()])
        .await
        .expect("add_block_headers");
    store
        .forkchoice_update(vec![(1, block1_hash)], 1, block1_hash, None, None)
        .await
        .expect("forkchoice_update");

    assert!(
        store
            .is_canonical_sync(block1_hash)
            .expect("is_canonical_sync"),
        "precondition: block 1 must be canonical"
    );
    assert!(
        !store.has_state_root(H256::zero()).expect("has_state_root"),
        "precondition: H256::zero() must not be a valid state root"
    );

    let result = is_resume_point(&store, &block1_header).expect("is_resume_point");
    assert!(
        !result,
        "a canonical-but-stateless header must not be a resume point"
    );
}

/// A header with present state (EMPTY_TRIE_HASH) but never made canonical must NOT be a
/// resume point: resume requires both canonicality and present state.
#[tokio::test]
async fn is_resume_point_stateful_noncanonical_is_false() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");

    let orphan_header = BlockHeader {
        number: 42,
        state_root: EMPTY_TRIE_HASH,
        ..Default::default()
    };

    assert!(
        store
            .has_state_root(EMPTY_TRIE_HASH)
            .expect("has_state_root"),
        "precondition: EMPTY_TRIE_HASH must always pass has_state_root"
    );
    assert!(
        !store
            .is_canonical_sync(orphan_header.hash())
            .expect("is_canonical_sync"),
        "precondition: orphan header must not be canonical"
    );

    let result = is_resume_point(&store, &orphan_header).expect("is_resume_point");
    assert!(
        !result,
        "a stateful-but-non-canonical header must not be a resume point"
    );
}
