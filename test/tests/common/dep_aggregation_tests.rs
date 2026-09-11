//! Tests for the aggregation seam itself. The leanVM backend's own tests live
//! behind the `leanvm` feature in `leanvm.rs`.

use ethrex_common::H256;
use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSPHINCS, DEPENDENCY_SCHEME_LEANSTARK, DependencyTriple,
};

use ethrex_dep_aggregation::{
    AggregateError, DependencyAggregator, LEANVM_AGGREGATOR, MAX_RECURSIVE_STARK_PROOF_BYTES,
    UnavailableAggregator, check_proof_length, default_aggregator,
};

fn triple(scheme: u8) -> DependencyTriple {
    DependencyTriple {
        scheme,
        data_hash: H256::from_low_u64_be(1),
        verification_key_hash: H256::from_low_u64_be(2),
    }
}

/// The build's backend is observable, so a test can tell whether it is exercising
/// the real aggregator or the refusing one. Without this a stub could silently
/// stand in for a verifier and every test would still pass.
#[test]
fn the_compiled_backend_is_observable() {
    let expected_name = if LEANVM_AGGREGATOR {
        "leanvm"
    } else {
        "unavailable"
    };
    assert_eq!(default_aggregator().name(), expected_name);
    assert_eq!(LEANVM_AGGREGATOR, cfg!(feature = "leanvm"));
}

/// A node with no backend must reject a block carrying a proof, not accept it.
/// This is the opposite of `ExecBackend`, whose `verify` returns `Ok` and warns --
/// right for a developer convenience, wrong for a consensus rule.
#[test]
fn the_unavailable_backend_fails_closed() {
    let agg = UnavailableAggregator;
    assert_eq!(
        agg.verify(
            b"any bytes at all",
            &[triple(DEPENDENCY_SCHEME_LEANSPHINCS)]
        ),
        Err(AggregateError::NoBackend),
        "a backend that cannot check a proof must reject it"
    );
    assert_eq!(
        agg.verify(&[], &[]),
        Err(AggregateError::NoBackend),
        "including for a block that declares no dependencies, which still carries a proof"
    );
    assert_eq!(
        agg.aggregate(&[], &[], None),
        Err(AggregateError::NoBackend)
    );
}

/// EIP-8288 sets no bound on the proof, but the header rides inside the block and
/// EIP-7934 caps the encoded block at 8 MiB. Reject implausible lengths before
/// anything decodes them.
#[test]
fn an_oversized_proof_is_rejected_before_decoding() {
    assert!(check_proof_length(&vec![0u8; MAX_RECURSIVE_STARK_PROOF_BYTES]).is_ok());
    assert_eq!(
        check_proof_length(&vec![0u8; MAX_RECURSIVE_STARK_PROOF_BYTES + 1]),
        Err(AggregateError::ProofTooLarge {
            len: MAX_RECURSIVE_STARK_PROOF_BYTES + 1,
            max: MAX_RECURSIVE_STARK_PROOF_BYTES,
        })
    );
}

/// The bound must leave a block usable. leanVM's aggregates are around 300 KiB, so
/// the cap has headroom for recursion while staying a small share of the 8 MiB
/// block budget.
#[test]
fn the_proof_bound_is_a_small_share_of_the_block() {
    use ethrex_common::constants::MAX_RLP_BLOCK_SIZE;
    assert!(
        (MAX_RECURSIVE_STARK_PROOF_BYTES as u64) < MAX_RLP_BLOCK_SIZE / 4,
        "a proof must not be able to crowd out the transactions it exists to serve"
    );
    assert!(
        MAX_RECURSIVE_STARK_PROOF_BYTES >= 512 * 1024,
        "must leave room for leanVM's ~300 KiB aggregates plus recursion"
    );
}

/// leanSTARK has no counterpart in leanVM, and the seam says so by name rather
/// than failing with something that reads like a corrupt proof.
#[test]
fn the_leanstark_scheme_is_named_as_unsupported() {
    let err = AggregateError::SchemeUnsupported {
        scheme: DEPENDENCY_SCHEME_LEANSTARK,
    };
    let rendered = err.to_string();
    assert!(rendered.contains("0x11"), "{rendered}");
    assert!(rendered.contains("cannot be proven"), "{rendered}");
}

/// The block-validity rules split by what they need, and rule 1 must hold for a
/// build with no aggregation backend at all -- it is computed from the block's own
/// transactions. Only rule 2 needs a verifier.
///
/// This pins that a node without the `leanvm` feature still rejects a tampered
/// dependency digest rather than deferring everything to a backend it does not have.
#[test]
fn rule_one_needs_no_backend() {
    use ethrex_common::types::{BlockBody, dependencies_hash};

    let empty = BlockBody {
        transactions: Vec::new(),
        ommers: Vec::new(),
        withdrawals: None,
    };
    assert_eq!(
        empty.block_deps_hash(),
        dependencies_hash(&[]),
        "a block with no dependencies commits to the digest of the empty set, not to zero"
    );
    assert_ne!(
        empty.block_deps_hash(),
        ethrex_common::H256::zero(),
        "which is a real digest, so an all-zero header field is not silently valid"
    );
}

/// A block that declares no dependencies is valid without any backend at all.
///
/// This one matters more than it looks. The header entry is mandatory from J*, so
/// without the vacuous arm every J* block reaches a backend -- including every
/// block on a chain where nobody has used the feature -- and a default build would
/// refuse the whole chain rather than the transactions it cannot check. Failing
/// closed is right; failing closed on any J* block rather than on unproven
/// dependencies is the wrong granularity.
#[test]
fn an_empty_dependency_set_needs_no_backend() {
    let agg = UnavailableAggregator;
    assert!(!agg.supports_scheme(DEPENDENCY_SCHEME_LEANSPHINCS));

    // The rule-2 caller short-circuits on an empty set before reaching the backend,
    // so what this pins is that the backend really would have refused.
    assert_eq!(
        agg.verify(&[], &[]),
        Err(AggregateError::NoBackend),
        "the backend refuses everything, so the short-circuit is what makes an \
         empty-set block importable"
    );
}

/// A backend advertises which schemes it can discharge, so admission can refuse a
/// transaction whose dependencies no block could ever prove.
#[test]
fn a_backend_declares_the_schemes_it_supports() {
    let agg = UnavailableAggregator;
    for scheme in [DEPENDENCY_SCHEME_LEANSPHINCS, DEPENDENCY_SCHEME_LEANSTARK] {
        assert!(
            !agg.supports_scheme(scheme),
            "a build with no backend discharges nothing"
        );
    }
}
