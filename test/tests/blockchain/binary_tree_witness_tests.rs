//! `debug_executionWitness` across the EIP-8297 activation boundary.
//!
//! The first section is the *diagnostic*: what the V1 handlers actually do
//! today when asked for a witness at a binary-committed block.
//!
//! `Blockchain::generate_witness_for_blocks_with_fee_configs` opens the first
//! block's parent through the **unchecked** `Store::state_trie`, which resolves
//! `header.state_root` against the MPT no matter which trie that root belongs
//! to. Past the flip that gives two different wrong answers depending on which
//! side of the boundary the *parent* sits on, and the dangerous one is the
//! success:
//!
//! 1. the **first** binary-committed block has a pre-flip parent, so the MPT
//!    open succeeds and V1 returns `Ok` with a complete, well-formed MPT
//!    witness over the parent's MPT state — a witness that answers a question
//!    nobody asked, since the header commits a binary root that no MPT witness
//!    can reproduce;
//! 2. any **later** block has a binary-committed parent, whose `state_root`
//!    names no MPT node at all, so the open fails with a confusing internal
//!    `Root node with hash ... not found` — reported as missing state for state
//!    the node holds, just in the other trie.

use ethrex_common::utils::keccak;
use ethrex_common::{H256, types::block_execution_witness::RpcExecutionWitness};
use ethrex_rpc::debug::execution_witness::ExecutionWitnessRequest;
use ethrex_rpc::debug::execution_witness_by_hash::ExecutionWitnessByBlockHashRequest;
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::types::block_identifier::BlockIdentifier;

use super::binary_tree_shadow_tests::{
    BoundaryChains, FLIP_BLOCK, binary_root, build_boundary_chains,
};

/// Assert the chain really flipped where these tests assume it did.
///
/// Every test below is vacuous against a chain that never activated the binary
/// commitment — a witness for an MPT block is supposed to work — so this is the
/// single place the whole file would go quiet, and it is checked, not assumed.
fn assert_flip_shape(chains: &BoundaryChains) {
    let head = chains.scheduled_blocks.last().expect("chain is non-empty");
    assert!(
        head.header.timestamp >= chains.activation,
        "the head must be past the activation, or the whole suite is vacuous"
    );
    assert_eq!(
        head.header.state_root,
        binary_root(&chains.scheduled_store, head),
        "the head must really commit a binary root"
    );
}

/// The hashes of every node in an `RpcExecutionWitness`'s `state` list.
///
/// An MPT witness is a flat list of node *encodings*, addressed by
/// `keccak(encoding)` — so this is the set of roots the witness can serve.
fn witness_node_hashes(value: &serde_json::Value) -> Vec<H256> {
    let witness: RpcExecutionWitness =
        serde_json::from_value(value.clone()).expect("V1 returns an RpcExecutionWitness");
    witness
        .state
        .iter()
        .map(|node| keccak(node))
        .collect()
}

// ---------------------------------------------------------------------------
// Step 1 — what V1 does today, post-flip.
// ---------------------------------------------------------------------------

/// The dangerous one: a plausible, well-formed witness for the wrong trie.
#[tokio::test]
async fn v1_witness_at_the_first_binary_block_silently_answers_for_the_parents_mpt() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let parent = chains.scheduled_blocks[chains.scheduled_blocks.len() - 2].clone();
    assert!(
        parent.header.timestamp < chains.activation,
        "this test needs the pre-flip parent that makes the MPT open succeed"
    );
    assert_ne!(
        head.header.state_root, parent.header.state_root,
        "the binary root and the parent's MPT root must differ, or nothing below \
         could tell them apart"
    );

    let context = default_context_with_storage(chains.scheduled_store.clone()).await;
    let response = ExecutionWitnessByBlockHashRequest {
        block_hash: head.hash(),
    }
    .handle(context)
    .await
    .expect("V1 succeeds here today — that is the finding");

    let hashes = witness_node_hashes(&response);
    assert!(
        hashes.contains(&parent.header.state_root),
        "the witness is rooted at the parent's MPT root: it is an MPT witness"
    );
    assert!(
        !hashes.contains(&head.header.state_root),
        "and it cannot contain the binary root the header actually commits to"
    );
}

/// The confusing one: a missing-state error for state the node holds.
#[tokio::test]
async fn v1_witness_past_the_first_binary_block_fails_as_a_missing_mpt_root() {
    let chains = build_boundary_chains(FLIP_BLOCK + 1).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let parent = chains.scheduled_blocks[chains.scheduled_blocks.len() - 2].clone();
    assert!(
        parent.header.timestamp >= chains.activation,
        "this test needs a binary-committed *parent*"
    );

    let context = default_context_with_storage(chains.scheduled_store.clone()).await;
    let error = ExecutionWitnessByBlockHashRequest {
        block_hash: head.hash(),
    }
    .handle(context)
    .await
    .expect_err("a binary-committed parent names no MPT root");

    let message = format!("{error:?}");
    assert!(
        message.contains("Root node") && message.contains("not found"),
        "the error is an MPT-internal one, not a statement about the binary \
         commitment: {message}"
    );
    // And it names the *binary* root as the missing MPT node, which is the
    // whole confusion: the node holds that state, in the other trie.
    assert!(
        message.contains(&format!("{:#x}", parent.header.state_root)),
        "the missing 'MPT root' is the parent's binary root: {message}"
    );
}

/// The by-number handler shares the generator, so it inherits both faults.
#[tokio::test]
async fn v1_witness_by_number_inherits_the_same_faults() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    make_canonical(&chains).await;

    let context = default_context_with_storage(chains.scheduled_store.clone()).await;
    let response = ExecutionWitnessRequest {
        from: BlockIdentifier::Number(head.header.number),
        to: None,
    }
    .handle(context)
    .await
    .expect("the by-number handler succeeds on the first binary block too");

    let parent = chains.scheduled_blocks[chains.scheduled_blocks.len() - 2].clone();
    assert!(
        witness_node_hashes(&response).contains(&parent.header.state_root),
        "same MPT witness, same wrong trie"
    );
}

/// Make the built chain canonical, which the by-number handler needs to resolve
/// a block number to a header at all.
async fn make_canonical(chains: &BoundaryChains) {
    let head = chains.scheduled_blocks.last().unwrap();
    ethrex_blockchain::fork_choice::apply_fork_choice(
        &chains.scheduled_store,
        head.hash(),
        head.hash(),
        head.hash(),
        None,
    )
    .await
    .expect("fork choice should apply to the built chain");
}
