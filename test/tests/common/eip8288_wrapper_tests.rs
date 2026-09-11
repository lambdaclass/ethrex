//! Tests for the EIP-8288 mempool wrapper object.

use ethrex_common::H256;
use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSPHINCS, DEPENDENCY_SCHEME_LEANSTARK, DependencyTriple,
    FRAME_TX_MAX_FRAMES, Frame, FrameMode, FrameTransaction,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

use ethrex_common::types::FRAME_TX_MAX_SIGS_PER_TX;
use ethrex_dep_aggregation::wrapper::{
    AGGREGATION_INTERVAL_MS, MAX_LEANSIG_DEPS_PER_WRAPPER, MAX_LEANSTARK_DEPS_PER_WRAPPER,
    MempoolWrapper, WrapperContent, WrapperEntry, WrapperError,
};
use ethrex_dep_aggregation::{DependencyAggregator, UnavailableAggregator};

fn dep(scheme: u8, n: u64) -> DependencyTriple {
    DependencyTriple {
        scheme,
        data_hash: H256::from_low_u64_be(n),
        verification_key_hash: H256::from_low_u64_be(n + 1000),
    }
}

fn sphincs(n: u64) -> DependencyTriple {
    dep(DEPENDENCY_SCHEME_LEANSPHINCS, n)
}

fn stark(n: u64) -> DependencyTriple {
    dep(DEPENDENCY_SCHEME_LEANSTARK, n)
}

fn wrapper_with(deps: Vec<DependencyTriple>) -> MempoolWrapper {
    MempoolWrapper {
        // A hash entry, so the union check short-circuits and the limit checks are
        // what the test is actually exercising.
        transactions: vec![WrapperEntry::Hash(H256::from_low_u64_be(1))],
        content: WrapperContent::Recursive {
            deps,
            recursive_stark: vec![0u8; 8],
        },
    }
}

#[test]
fn wrapper_constants_match_the_published_table() {
    assert_eq!(MAX_LEANSIG_DEPS_PER_WRAPPER, 16);
    assert_eq!(MAX_LEANSTARK_DEPS_PER_WRAPPER, 1);
    assert_eq!(AGGREGATION_INTERVAL_MS, 1_000);
}

/// The wrapper's own limits, checked before anything expensive runs.
#[test]
fn the_wrapper_limits_are_enforced() {
    let agg = UnavailableAggregator;

    let too_many: Vec<_> = (0..=MAX_LEANSIG_DEPS_PER_WRAPPER as u64)
        .map(sphincs)
        .collect();
    assert_eq!(
        wrapper_with(too_many).validate(&agg),
        Err(WrapperError::TooManyLeanSphincs {
            count: MAX_LEANSIG_DEPS_PER_WRAPPER + 1,
            limit: MAX_LEANSIG_DEPS_PER_WRAPPER,
        })
    );

    let too_many_starks: Vec<_> = (0..=MAX_LEANSTARK_DEPS_PER_WRAPPER as u64)
        .map(stark)
        .collect();
    assert_eq!(
        wrapper_with(too_many_starks).validate(&agg),
        Err(WrapperError::TooManyLeanStark {
            count: MAX_LEANSTARK_DEPS_PER_WRAPPER + 1,
            limit: MAX_LEANSTARK_DEPS_PER_WRAPPER,
        })
    );
}

/// The wrapper limits, pinned as an assertion rather than only as prose.
///
/// EIP-8288 exists to aggregate many signatures into one proof -- its Motivation
/// talks about "many thousands of signatures per slot", and the tooling it names
/// benchmarks 245 SPHINCS signatures per aggregate. But a wrapper may carry at most
/// `MAX_LEANSIG_DEPS_PER_WRAPPER` (16) leanSPHINCS dependencies, while a single
/// transaction may declare `MAX_SIGS_PER_TX` (16). So one maximally-loaded
/// transaction fills a whole wrapper.
///
/// Lighter transactions still aggregate together -- sixteen declaring one signature
/// each fit. What the limit fixes is the ceiling: no wrapper ever carries more than
/// 16 signatures however they are spread, an order of magnitude below what the
/// tooling proves in one aggregate.
///
/// If the limits are raised upstream this test fails, which is the point: it should
/// be looked at rather than silently tracking a number.
#[test]
fn one_transaction_can_fill_an_entire_wrapper() {
    assert_eq!(
        MAX_LEANSIG_DEPS_PER_WRAPPER, FRAME_TX_MAX_SIGS_PER_TX,
        "a wrapper holds exactly one maximally-loaded transaction's worth of \
         leanSPHINCS dependencies, so the per-wrapper ceiling is sized against one \
         transaction rather than against what an aggregate can hold"
    );
    assert_eq!(
        MAX_LEANSTARK_DEPS_PER_WRAPPER, 1,
        "and at most one leanSTARK dependency may propagate per aggregation round"
    );
    // Not a limit anyone hits: the per-frame cap is far above what a wrapper admits.
    assert!(FRAME_TX_MAX_FRAMES > MAX_LEANSIG_DEPS_PER_WRAPPER);
}

#[test]
fn a_wrapper_with_no_transactions_is_rejected() {
    let agg = UnavailableAggregator;
    let w = MempoolWrapper {
        transactions: Vec::new(),
        content: WrapperContent::Recursive {
            deps: vec![sphincs(1)],
            recursive_stark: vec![0u8; 4],
        },
    };
    assert_eq!(w.validate(&agg), Err(WrapperError::Empty));
}

/// Rule 1 of both modes is that `deps` is the union of the transactions'
/// dependencies, which a receiver has to resolve the hash to check. A node with a
/// pool of already-broadcast wrappers should resolve first; this implementation has
/// no wrapper transport and so nothing to resolve against, and the EIP defines no
/// behaviour for a miss either way.
#[test]
fn a_hash_only_wrapper_cannot_have_its_union_checked() {
    let agg = UnavailableAggregator;
    assert_eq!(
        wrapper_with(vec![sphincs(1)]).validate(&agg),
        Err(WrapperError::UnresolvedHashes),
        "the union rule is unenforceable against a hash, so it must not silently pass"
    );
}

/// A transaction declaring exactly `deps`, so a wrapper carrying it gets past the
/// union check and the rules after it can be reached.
fn tx_declaring(deps: &[DependencyTriple]) -> FrameTransaction {
    let mut data = Vec::new();
    for d in deps {
        data.extend_from_slice(&d.encode());
    }
    FrameTransaction {
        frames: vec![Frame {
            mode: FrameMode::DepVerify as u8,
            data: bytes::Bytes::from(data),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn mode_zero_needs_one_proof_per_dependency() {
    let agg = UnavailableAggregator;
    let deps = vec![sphincs(1), sphincs(2)];
    let w = MempoolWrapper {
        transactions: vec![WrapperEntry::Full(Box::new(tx_declaring(&deps)))],
        content: WrapperContent::Direct {
            deps: deps.clone(),
            proofs: vec![vec![0u8; 4]],
        },
    };
    assert_eq!(
        w.validate(&agg),
        Err(WrapperError::ProofCountMismatch { proofs: 1, deps: 2 }),
        "the count rule has to be what rejects this, not the union check upstream of it"
    );
}

/// "Each dependency has a corresponding proof that can be verified individually."
/// Individually means against that dependency's own scheme -- so mode 0's proofs go
/// to `verify_witness`, never to `verify`, whose subject is an aggregate over a
/// whole expected set.
#[test]
fn mode_zero_verifies_each_dependency_on_its_own_terms() {
    let agg = UnavailableAggregator;
    for deps in [vec![sphincs(1)], vec![stark(1)]] {
        let w = MempoolWrapper {
            transactions: vec![WrapperEntry::Full(Box::new(tx_declaring(&deps)))],
            content: WrapperContent::Direct {
                deps: deps.clone(),
                proofs: vec![vec![0u8; 4]],
            },
        };
        // `UnavailableAggregator` refuses both entry points, so what this pins is
        // that validation reaches a per-dependency check at all rather than
        // short-circuiting earlier.
        assert_eq!(
            w.validate(&agg),
            Err(WrapperError::Aggregate(
                ethrex_dep_aggregation::AggregateError::NoBackend
            )),
            "scheme {:#04x} must reach a per-dependency check",
            deps[0].scheme
        );
    }
}

#[test]
fn a_wrapper_round_trips_through_rlp() {
    for content in [
        WrapperContent::Recursive {
            deps: vec![sphincs(1), stark(2)],
            recursive_stark: vec![7u8; 32],
        },
        WrapperContent::Direct {
            deps: vec![sphincs(1), sphincs(2)],
            proofs: vec![vec![1u8; 8], vec![2u8; 8]],
        },
    ] {
        let w = MempoolWrapper {
            transactions: vec![
                WrapperEntry::Hash(H256::from_low_u64_be(0xAA)),
                WrapperEntry::Hash(H256::from_low_u64_be(0xBB)),
            ],
            content,
        };
        let mut buf = Vec::new();
        w.encode(&mut buf);
        assert_eq!(MempoolWrapper::decode(&buf).unwrap(), w);
    }
}

#[test]
fn an_unknown_wrapper_mode_is_rejected() {
    // Built by hand rather than by mutating a valid encoding: only modes 0 and 1
    // are assigned, and a decoder must not guess at a third.
    let mut buf = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut buf)
        .encode_field(&vec![H256::from_low_u64_be(1)])
        .encode_field(&2u64)
        .encode_field(&(Vec::<Vec<u8>>::new(), Vec::<u8>::new()))
        .finish();
    assert!(
        MempoolWrapper::decode(&buf).is_err(),
        "an unassigned mode must not decode as one of the two the EIP defines"
    );
}

/// A build with no backend must not accept a wrapper's proofs, for the same reason
/// it must not accept a block's.
#[test]
fn wrapper_validation_fails_closed_without_a_backend() {
    let agg = UnavailableAggregator;
    let w = MempoolWrapper {
        transactions: vec![WrapperEntry::Full(Box::default())],
        content: WrapperContent::Recursive {
            deps: Vec::new(),
            recursive_stark: vec![0u8; 4],
        },
    };
    // An empty default transaction declares no dependencies, so the union check
    // passes and validation reaches the proof.
    assert!(matches!(
        w.validate(&agg),
        Err(WrapperError::Aggregate(
            ethrex_dep_aggregation::AggregateError::NoBackend
        ))
    ));
    assert_eq!(agg.name(), "unavailable");
}
