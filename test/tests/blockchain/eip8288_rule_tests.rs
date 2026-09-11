//! EIP-8288 block-validity rule 2, and the two outcomes it can produce.
//!
//! Rule 1 -- the header's digest against the block's own transactions -- needs no
//! backend and is tested in `common::eip8288_tests`. This file covers the half that
//! does: whether the recursive proof discharges what the block declared, and what a
//! node says when it cannot tell.

use bytes::Bytes;
use ethrex_blockchain::eip8288::validate_recursive_stark;
use ethrex_blockchain::error::{ChainError, InvalidForkChoice};
use ethrex_blockchain::fork_choice::map_chain_error_for_fcu;
use ethrex_common::H256;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, ChainConfig, DEPENDENCY_SCHEME_LEANSPHINCS, DependencyTriple,
    Frame, FrameMode, FrameTransaction, RecursiveStark, Transaction,
};
use ethrex_dep_aggregation::{
    AggregateError, DependencyAggregator, DependencyWitness, UnavailableAggregator,
};

fn jstar_config() -> ChainConfig {
    ChainConfig {
        jstar_time: Some(0),
        ..Default::default()
    }
}

/// A block whose single transaction declares one leanSPHINCS dependency.
fn block_with_a_dependency(proof: Bytes) -> Block {
    let triple = DependencyTriple {
        scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
        data_hash: H256::from_low_u64_be(1),
        verification_key_hash: H256::from_low_u64_be(2),
    };
    let tx = FrameTransaction {
        frames: vec![Frame {
            mode: FrameMode::DepVerify as u8,
            data: Bytes::from(triple.encode().to_vec()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let body = BlockBody {
        transactions: vec![Transaction::FrameTransaction(tx)],
        ..Default::default()
    };
    let header = BlockHeader {
        recursive_stark: Some(RecursiveStark {
            proof,
            block_deps_hash: body.block_deps_hash(),
        }),
        ..Default::default()
    };
    Block::new(header, body)
}

fn empty_block(proof: Bytes) -> Block {
    let body = BlockBody::default();
    let header = BlockHeader {
        recursive_stark: Some(RecursiveStark {
            proof,
            block_deps_hash: body.block_deps_hash(),
        }),
        ..Default::default()
    };
    Block::new(header, body)
}

/// The arm that decides whether a default build can follow a J* chain at all. Every
/// block carries the header entry from J* on, so without this a node with no backend
/// would refuse the whole chain rather than the transactions it cannot check.
#[test]
fn a_block_with_no_dependencies_needs_no_backend() {
    validate_recursive_stark(
        &empty_block(Bytes::new()),
        &jstar_config(),
        &UnavailableAggregator,
    )
    .expect("nothing to discharge means nothing to verify");
}

/// And the shortcut is narrow: a block declaring nothing but carrying a proof anyway
/// is checked like any other, not waved through.
#[test]
fn a_proof_on_an_empty_block_is_still_checked() {
    assert!(matches!(
        validate_recursive_stark(
            &empty_block(Bytes::from_static(b"padding")),
            &jstar_config(),
            &UnavailableAggregator,
        ),
        Err(ChainError::RecursiveStarkUnverifiable(_))
    ));
}

/// A node with no verifier has learned nothing about the block. It must refuse it,
/// but as a local incapacity -- the two errors reach fork choice differently.
#[test]
fn a_dependency_without_a_backend_is_unverifiable_not_invalid() {
    assert!(matches!(
        validate_recursive_stark(
            &block_with_a_dependency(Bytes::from_static(b"an aggregate")),
            &jstar_config(),
            &UnavailableAggregator,
        ),
        Err(ChainError::RecursiveStarkUnverifiable(_))
    ));
}

/// A backend that has the machinery but not this scheme. leanSTARK (`0x11`) is the
/// real case: statically valid, and no backend can express it.
#[derive(Debug)]
struct SchemeBlindAggregator;

impl DependencyAggregator for SchemeBlindAggregator {
    fn verify(&self, _proof: &[u8], expected: &[DependencyTriple]) -> Result<(), AggregateError> {
        Err(AggregateError::SchemeUnsupported {
            scheme: expected[0].scheme,
        })
    }

    fn aggregate(
        &self,
        _raw: &[DependencyWitness],
        _children: &[&[u8]],
        _declare: Option<&[DependencyTriple]>,
    ) -> Result<Vec<u8>, AggregateError> {
        unimplemented!("not exercised")
    }

    fn verify_witness(&self, _witness: &DependencyWitness) -> Result<(), AggregateError> {
        unimplemented!("not exercised")
    }

    fn supports_scheme(&self, _scheme: u8) -> bool {
        false
    }

    fn aggregated_vk(&self) -> H256 {
        H256::zero()
    }

    fn name(&self) -> &'static str {
        "scheme-blind"
    }
}

/// The same reasoning as having no backend, and easy to get wrong: an unsupported
/// scheme is a fact about this node's backend, not about the block. A node that
/// reported INVALID here would tell its consensus client that a chain the rest of
/// the network follows is bad.
#[test]
fn an_unsupported_scheme_is_unverifiable_not_invalid() {
    assert!(matches!(
        validate_recursive_stark(
            &block_with_a_dependency(Bytes::from_static(b"an aggregate")),
            &jstar_config(),
            &SchemeBlindAggregator,
        ),
        Err(ChainError::RecursiveStarkUnverifiable(_))
    ));
}

/// The rule is fork-gated, so a pre-J* block never reaches a backend.
#[test]
fn the_rule_is_silent_before_jstar() {
    validate_recursive_stark(
        &block_with_a_dependency(Bytes::from_static(b"an aggregate")),
        &ChainConfig::default(),
        &UnavailableAggregator,
    )
    .expect("rule 2 does not apply before J*");
}

/// A schedule naming LStar and no `jstarTime` resolves to `Fork::LStar`, which the
/// VM's frame-mode gate reads as "mode 3 is legal". Rule 2 has to read it the same
/// way, or that chain runs dependency frames nothing ever discharges.
#[test]
fn a_schedule_past_jstar_without_a_jstar_time_still_gets_the_rule() {
    let lstar_only = ChainConfig {
        lstar_time: Some(0),
        ..Default::default()
    };
    assert!(
        !lstar_only.is_jstar_activated(0),
        "the timestamp predicate says no, which is the trap this guards"
    );
    assert!(lstar_only.is_jstar_or_later(0), "the ordinal says yes");

    assert!(matches!(
        validate_recursive_stark(
            &block_with_a_dependency(Bytes::from_static(b"an aggregate")),
            &lstar_only,
            &UnavailableAggregator,
        ),
        Err(ChainError::RecursiveStarkUnverifiable(_))
    ));
}

/// Defensive: header validation rejects this first, but rule 2 must not read a
/// missing entry as "no dependencies to discharge".
#[test]
fn a_jstar_block_missing_the_entry_is_invalid() {
    let mut block = block_with_a_dependency(Bytes::new());
    block.header.recursive_stark = None;
    assert!(matches!(
        validate_recursive_stark(&block, &jstar_config(), &UnavailableAggregator),
        Err(ChainError::RecursiveStarkInvalid(_))
    ));
}

/// The two errors exist to be told apart here: "this block is wrong" and "this node
/// cannot tell" are different answers to give a consensus client, and collapsing
/// them would have a node with no aggregation backend declare a chain everyone else
/// follows invalid.
#[test]
fn the_two_recursive_stark_errors_map_to_different_fork_choice_outcomes() {
    let last_valid = H256::from_low_u64_be(0xFEED);

    assert!(matches!(
        map_chain_error_for_fcu(
            ChainError::RecursiveStarkInvalid("proof does not discharge".to_string()),
            last_valid,
        ),
        InvalidForkChoice::InvalidAncestor(h) if h == last_valid
    ));

    assert!(matches!(
        map_chain_error_for_fcu(
            ChainError::RecursiveStarkUnverifiable("no backend".to_string()),
            last_valid,
        ),
        InvalidForkChoice::StateNotReachable
    ));
}
