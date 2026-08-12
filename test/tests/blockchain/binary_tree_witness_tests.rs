//! `debug_executionWitness` / `debug_executionWitnessV2` across the EIP-8297
//! activation boundary.
//!
//! # Section 1 — what the unguarded V1 path does past the flip
//!
//! `Blockchain::generate_witness_for_blocks` opens the first block's parent
//! through the **unchecked** `Store::state_trie`, which resolves
//! `header.state_root` against the MPT no matter which trie that root belongs
//! to. Past the flip that gives two different wrong answers, decided by which
//! side of the boundary the *parent* sits on, and the dangerous one is the
//! success:
//!
//! 1. the **first** binary-committed block has a pre-flip parent, so the MPT
//!    open succeeds and the generator returns a complete, well-formed MPT
//!    witness over the parent's MPT state — a witness that answers a question
//!    nobody asked, since the header commits a binary root that no MPT witness
//!    can reproduce. `state_trie_checked` would *not* have caught this: the
//!    parent's MPT state really is held;
//! 2. any **later** block has a binary-committed parent, whose `state_root`
//!    names no MPT node at all, so the open fails with a confusing internal
//!    `Root node with hash ... not found` — reported as missing state for state
//!    the node holds, just in the other trie.
//!
//! These tests call the generator directly, because the RPC handlers now refuse
//! before reaching it; they are what the guard in section 2 is guarding against,
//! and they must keep failing this way for the guard to be load-bearing.
//!
//! # Section 2 — the guarded pair
//!
//! V1 refuses binary-committed headers and points at V2; V2 refuses
//! MPT-committed ones and points back at V1. **Per header, never per chain**: a
//! pre-activation block on a scheduled chain keeps its MPT witness forever.

use ethrex_blockchain::Blockchain;
use ethrex_common::utils::keccak;
use ethrex_common::{H256, types::block_execution_witness::RpcExecutionWitness};
use ethrex_rpc::debug::execution_witness::ExecutionWitnessRequest;
use ethrex_rpc::debug::execution_witness_by_hash::ExecutionWitnessByBlockHashRequest;
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::types::block_identifier::BlockIdentifier;
use ethrex_rpc::utils::RpcErr;

use super::binary_tree_shadow_tests::{
    BoundaryChains, FLIP_BLOCK, binary_root, build_boundary_chains,
};

/// Assert the chain really flipped where these tests assume it did.
///
/// Every test here is vacuous against a chain that never activated the binary
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
fn witness_node_hashes(witness: &RpcExecutionWitness) -> Vec<H256> {
    witness.state.iter().map(|node| keccak(node)).collect()
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

// ---------------------------------------------------------------------------
// Section 1 — the unguarded generator, past the flip.
// ---------------------------------------------------------------------------

/// The dangerous one: a plausible, well-formed witness for the wrong trie.
#[tokio::test]
async fn the_mpt_generator_at_the_first_binary_block_answers_for_the_parents_mpt() {
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

    let blockchain = Blockchain::default_with_store(chains.scheduled_store.clone());
    let witness = blockchain
        .generate_witness_for_blocks(std::slice::from_ref(&head))
        .await
        .expect("the MPT generator succeeds here today — that is the finding");
    let witness = RpcExecutionWitness::try_from(witness).expect("witness converts to RPC form");

    let hashes = witness_node_hashes(&witness);
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
async fn the_mpt_generator_past_the_first_binary_block_fails_as_a_missing_mpt_root() {
    let chains = build_boundary_chains(FLIP_BLOCK + 1).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let parent = chains.scheduled_blocks[chains.scheduled_blocks.len() - 2].clone();
    assert!(
        parent.header.timestamp >= chains.activation,
        "this test needs a binary-committed *parent*"
    );

    let blockchain = Blockchain::default_with_store(chains.scheduled_store.clone());
    let Err(error) = blockchain
        .generate_witness_for_blocks(std::slice::from_ref(&head))
        .await
    else {
        panic!("a binary-committed parent names no MPT root, so this must fail");
    };

    let message = format!("{error}");
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

// ---------------------------------------------------------------------------
// Section 2 — the guard.
// ---------------------------------------------------------------------------

fn assert_points_at_v2(error: &RpcErr) {
    let message = format!("{error:?}");
    assert!(
        matches!(error, RpcErr::UnsupportedFork(_)),
        "the refusal must be an UnsupportedFork, not an internal error: {message}"
    );
    assert!(
        message.contains("debug_executionWitnessV2"),
        "the refusal must name the method that does answer: {message}"
    );
}

#[tokio::test]
async fn v1_by_hash_refuses_a_binary_committed_header() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();

    let context = default_context_with_storage(chains.scheduled_store.clone()).await;
    let error = ExecutionWitnessByBlockHashRequest {
        block_hash: head.hash(),
    }
    .handle(context)
    .await
    .expect_err("V1 must refuse a binary-committed header");
    assert_points_at_v2(&error);
}

#[tokio::test]
async fn v1_by_number_refuses_a_binary_committed_header() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    make_canonical(&chains).await;
    let head = chains.scheduled_blocks.last().unwrap().clone();

    let context = default_context_with_storage(chains.scheduled_store.clone()).await;
    let error = ExecutionWitnessRequest {
        from: BlockIdentifier::Number(head.header.number),
        to: None,
    }
    .handle(context)
    .await
    .expect_err("V1 must refuse a binary-committed header");
    assert_points_at_v2(&error);
}

/// The per-header rule, and the falsification target for a per-chain guard: on
/// a chain that *has* flipped, every pre-activation block must still get its
/// MPT witness out of V1.
#[tokio::test]
async fn v1_still_serves_pre_activation_blocks_on_a_flipped_chain() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let context = default_context_with_storage(chains.scheduled_store.clone()).await;

    let mut served = 0;
    for block in &chains.scheduled_blocks {
        if block.header.timestamp >= chains.activation {
            continue;
        }
        let response = ExecutionWitnessByBlockHashRequest {
            block_hash: block.hash(),
        }
        .handle(context.clone())
        .await
        .unwrap_or_else(|error| {
            panic!(
                "pre-activation block {} must keep its MPT witness after the flip, got {error:?}",
                block.header.number
            )
        });
        let witness: RpcExecutionWitness =
            serde_json::from_value(response).expect("V1 returns an RpcExecutionWitness");
        assert!(
            !witness.state.is_empty(),
            "block {} must get a non-empty MPT witness",
            block.header.number
        );
        served += 1;
    }
    assert_eq!(
        served,
        FLIP_BLOCK as usize - 1,
        "the chain must really contain pre-activation blocks to serve"
    );
}

/// A range spanning the boundary is refused, and the refusal names the *first*
/// binary-committed block in it rather than the range's endpoint.
#[tokio::test]
async fn v1_by_number_refuses_a_range_that_crosses_the_boundary() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    make_canonical(&chains).await;
    let head = chains.scheduled_blocks.last().unwrap().clone();

    let context = default_context_with_storage(chains.scheduled_store.clone()).await;
    let error = ExecutionWitnessRequest {
        from: BlockIdentifier::Number(1),
        to: Some(BlockIdentifier::Number(head.header.number)),
    }
    .handle(context)
    .await
    .expect_err("a range reaching past the activation must be refused");
    assert_points_at_v2(&error);
    assert!(
        format!("{error:?}").contains(&format!("block {}", head.header.number)),
        "the refusal must name the offending block: {error:?}"
    );
}
