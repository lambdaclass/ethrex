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
//!
//! # Section 3 — the V2 witness re-executes
//!
//! The test that makes the rest worth having: take the V2 witness for a
//! binary-committed block and, *from that alone*, recompute the post-state root
//! and get the header's value. Then break it four ways — a missing node, a
//! corrupted node, a witness from a different block, a node that does not
//! belong — and show each breakage is caught.

use ethrex_blockchain::Blockchain;
use ethrex_blockchain::binary_witness::recompute_post_state_root;
use ethrex_common::types::Block;
use ethrex_common::types::binary_execution_witness::{
    BINARY_WITNESS_FORMAT, RpcBinaryExecutionWitness,
};
use ethrex_common::utils::keccak;
use ethrex_common::{H256, types::block_execution_witness::RpcExecutionWitness};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::debug::execution_witness::ExecutionWitnessRequest;
use ethrex_rpc::debug::execution_witness_by_hash::ExecutionWitnessByBlockHashRequest;
use ethrex_rpc::debug::execution_witness_v2::{
    ExecutionWitnessV2ByBlockHashRequest, ExecutionWitnessV2Request,
};
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

// ---------------------------------------------------------------------------
// Section 3 — the V2 witness, and whether anything can re-execute it.
// ---------------------------------------------------------------------------

/// The V2 witness for `block`, straight out of the RPC handler.
async fn v2_witness(chains: &BoundaryChains, block: &Block) -> RpcBinaryExecutionWitness {
    let context = default_context_with_storage(chains.scheduled_store.clone()).await;
    let response = ExecutionWitnessV2ByBlockHashRequest {
        block_hash: block.hash(),
    }
    .handle(context)
    .await
    .unwrap_or_else(|error| panic!("V2 must serve block {}: {error:?}", block.header.number));
    serde_json::from_value(response).expect("V2 returns an RpcBinaryExecutionWitness")
}

/// Recompute `block`'s post-state root from `witness` and nothing else.
fn replay(
    witness: &RpcBinaryExecutionWitness,
    block: &Block,
    chains: &BoundaryChains,
) -> Result<H256, String> {
    recompute_post_state_root(
        witness,
        std::slice::from_ref(block),
        chains.scheduled_genesis.config,
    )
    .map_err(|error| format!("{error}"))
}

/// The whole point. No store, no disk: the witness plus the block.
#[tokio::test]
async fn a_v2_witness_re_executes_to_the_committed_binary_root() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let witness = v2_witness(&chains, &head).await;

    assert_eq!(witness.format, BINARY_WITNESS_FORMAT);
    assert!(
        !witness.state.is_empty(),
        "a witness with no nodes could not prove anything"
    );
    // Non-vacuity: the root reproduced is a binary root, and it is not the
    // pre-state root, so reproducing it is a statement about this block.
    assert_ne!(
        witness.pre_state_root, head.header.state_root,
        "the block must change the state, or the replay proves nothing"
    );

    assert_eq!(
        replay(&witness, &head, &chains).expect("the witness must re-execute"),
        head.header.state_root,
        "the witness must reproduce the root the header commits"
    );
}

/// The deeper case, where the parent is itself binary-committed — so
/// `preStateRoot` equals the parent header's `state_root` rather than a
/// shadow-tracked value.
#[tokio::test]
async fn a_v2_witness_re_executes_past_the_first_binary_block() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let parent = chains.scheduled_blocks[chains.scheduled_blocks.len() - 2].clone();
    assert!(parent.header.timestamp >= chains.activation);

    let witness = v2_witness(&chains, &head).await;
    assert_eq!(
        witness.pre_state_root, parent.header.state_root,
        "a binary-committed parent carries the pre-state root in its own header"
    );
    assert_eq!(
        replay(&witness, &head, &chains).expect("the witness must re-execute"),
        head.header.state_root
    );
}

/// The first binary block's parent is pre-flip, so its header commits an MPT
/// root and the binary pre-state root appears in no header at all. This is the
/// case that forces `preStateRoot` into the wire format.
#[tokio::test]
async fn the_first_binary_blocks_pre_state_root_is_in_no_header() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let parent = chains.scheduled_blocks[chains.scheduled_blocks.len() - 2].clone();
    assert!(parent.header.timestamp < chains.activation);

    let witness = v2_witness(&chains, &head).await;
    assert_ne!(
        witness.pre_state_root, parent.header.state_root,
        "the parent header commits an MPT root; the binary pre-state is elsewhere"
    );
    assert!(!witness.headers.is_empty(), "the witness carries headers");
    for encoded in &witness.headers {
        let header =
            ethrex_common::types::BlockHeader::decode(encoded).expect("witness headers decode");
        assert_ne!(
            header.state_root, witness.pre_state_root,
            "no header in the witness carries the binary pre-state root"
        );
    }
}

// --- and now break it four ways --------------------------------------------

#[tokio::test]
async fn a_v2_witness_missing_a_node_does_not_re_execute() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let witness = v2_witness(&chains, &head).await;
    assert!(witness.state.len() > 2, "there must be nodes to drop");

    for drop in 0..witness.state.len() {
        let mut broken = witness.clone();
        broken.state.remove(drop);
        if let Ok(root) = replay(&broken, &head, &chains) {
            assert_ne!(
                root, head.header.state_root,
                "dropping node {drop} still reproduced the committed root"
            );
            panic!("dropping node {drop} produced a root instead of an error: {root:#x}");
        }
    }
}

#[tokio::test]
async fn a_v2_witness_with_a_corrupted_node_does_not_re_execute() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let witness = v2_witness(&chains, &head).await;

    // A node's bytes are its BLAKE3 preimage, so any change unnames it.
    for index in 0..witness.state.len() {
        let len = witness.state[index].len();
        for byte in [0usize, len / 2, len - 1] {
            let mut broken = witness.clone();
            let mut bytes = broken.state[index].to_vec();
            bytes[byte] ^= 1;
            broken.state[index] = bytes.into();
            if let Ok(root) = replay(&broken, &head, &chains) {
                panic!(
                    "flipping byte {byte} of node {index} still produced a root: {root:#x} \
                     (committed {:#x})",
                    head.header.state_root
                );
            }
        }
    }
}

#[tokio::test]
async fn a_v2_witness_for_another_block_does_not_re_execute() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let other = chains.scheduled_blocks[chains.scheduled_blocks.len() - 2].clone();
    assert!(other.header.timestamp >= chains.activation);
    assert_ne!(head.hash(), other.hash());

    let other_witness = v2_witness(&chains, &other).await;
    // Sanity: it does verify for its own block, so the failure below is about
    // the pairing and not about the witness being broken.
    assert_eq!(
        replay(&other_witness, &other, &chains).expect("its own block re-executes"),
        other.header.state_root
    );

    if let Ok(root) = replay(&other_witness, &head, &chains) {
        assert_ne!(
            root, head.header.state_root,
            "another block's witness reproduced this block's root"
        );
    }
}

/// An otherwise-perfect witness with one extra node bolted on.
///
/// The extra node is a *well-formed* node — a real one with a byte changed — so
/// it decodes fine and is not caught by anything structural. What catches it is
/// that its hash is now different from every child pointer in the witness, so
/// the downward walk never reaches it.
///
/// Distinct from the corruption test above, which *replaces* a node: here every
/// original node is still present, so the witness would verify but for the
/// passenger.
#[tokio::test]
async fn a_v2_witness_with_a_node_that_does_not_belong_is_rejected() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let witness = v2_witness(&chains, &head).await;

    let mut stranger = witness.state[0].to_vec();
    let last = stranger.len() - 1;
    stranger[last] ^= 1;
    assert!(
        !witness.state.iter().any(|node| node.as_ref() == stranger),
        "the fabricated node must not already be in the witness"
    );

    // Unpadded, it verifies — so the failure below is caused by the passenger
    // and by nothing else.
    assert_eq!(
        replay(&witness, &head, &chains).expect("the untouched witness re-executes"),
        head.header.state_root
    );

    let mut padded = witness.clone();
    padded.state.push(stranger.into());
    let error = replay(&padded, &head, &chains)
        .expect_err("a node nothing in the witness names must be rejected");
    assert!(
        error.contains("nothing in it names"),
        "the rejection must say why: {error}"
    );
}

/// The discriminator does work: a wrong `format` is refused before anything is
/// decoded, rather than indexing into a tree with no nodes anything names.
#[tokio::test]
async fn a_witness_in_the_wrong_format_is_refused_by_name() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let head = chains.scheduled_blocks.last().unwrap().clone();
    let mut witness = v2_witness(&chains, &head).await;
    witness.format = "something-else".to_string();

    let error = replay(&witness, &head, &chains).expect_err("a wrong format must be refused");
    assert!(
        error.contains(BINARY_WITNESS_FORMAT),
        "the refusal must name the format it wanted: {error}"
    );
}

// --- the guard, from V2's side ---------------------------------------------

#[tokio::test]
async fn v2_refuses_a_pre_activation_header_and_points_back_at_v1() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    let context = default_context_with_storage(chains.scheduled_store.clone()).await;

    let mut refused = 0;
    for block in &chains.scheduled_blocks {
        if block.header.timestamp >= chains.activation {
            continue;
        }
        let error = ExecutionWitnessV2ByBlockHashRequest {
            block_hash: block.hash(),
        }
        .handle(context.clone())
        .await
        .expect_err("V2 must refuse an MPT-committed header");
        let message = format!("{error:?}");
        assert!(
            matches!(error, RpcErr::UnsupportedFork(_)),
            "the refusal must be an UnsupportedFork: {message}"
        );
        assert!(
            message.contains("Use debug_executionWitness"),
            "the refusal must point back at V1: {message}"
        );
        refused += 1;
    }
    assert_eq!(
        refused,
        FLIP_BLOCK as usize - 1,
        "the chain must really contain pre-activation blocks to refuse"
    );
}

/// A chain with no `binaryTreeTime` at all: every block is MPT-committed, so V2
/// answers for none of them. The chain-level and per-header questions agree
/// here, which is why this cannot be the only guard test.
#[tokio::test]
async fn v2_refuses_every_block_of_an_unscheduled_chain() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    let context = default_context_with_storage(chains.twin_store.clone()).await;
    assert!(!chains.twin_blocks.is_empty());
    for block in &chains.twin_blocks {
        let error = ExecutionWitnessV2ByBlockHashRequest {
            block_hash: block.hash(),
        }
        .handle(context.clone())
        .await
        .expect_err("V2 must refuse a chain with no binary commitment");
        assert!(matches!(error, RpcErr::UnsupportedFork(_)));
    }
}

#[tokio::test]
async fn v2_by_number_serves_a_binary_committed_header() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    assert_flip_shape(&chains);
    make_canonical(&chains).await;
    let head = chains.scheduled_blocks.last().unwrap().clone();

    let context = default_context_with_storage(chains.scheduled_store.clone()).await;
    let response = ExecutionWitnessV2Request {
        from: BlockIdentifier::Number(head.header.number),
        to: None,
    }
    .handle(context)
    .await
    .expect("V2 must serve a binary-committed header by number");
    let witness: RpcBinaryExecutionWitness =
        serde_json::from_value(response).expect("V2 returns an RpcBinaryExecutionWitness");
    assert_eq!(
        replay(&witness, &head, &chains).expect("the by-number witness re-executes"),
        head.header.state_root
    );
}
