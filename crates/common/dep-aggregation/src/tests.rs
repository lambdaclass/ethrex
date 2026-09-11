//! Tests for the aggregation seam itself. The leanVM backend's own tests live
//! behind the `leanvm` feature in `leanvm.rs`.

use ethrex_common::H256;
use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSPHINCS, DEPENDENCY_SCHEME_LEANSTARK, DependencyTriple,
};

use crate::{
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
    assert_eq!(agg.aggregate(&[], &[]), Err(AggregateError::NoBackend));
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
