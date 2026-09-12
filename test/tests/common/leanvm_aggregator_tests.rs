//! The leanVM aggregation backend, exercised for real.
//!
//! Only compiled with the `leanvm` feature, and the proving tests are `#[ignore]`d
//! within that: producing an aggregate peaks around 9-11 GiB of resident memory and
//! takes about a second. Run them deliberately:
//! `cargo test -p ethrex-test --features leanvm -- --ignored leanvm_aggregator`
//!
//! What these establish is that the cryptography works -- signatures are real,
//! aggregates are real, and a proof over the wrong set fails. What they do not
//! establish is conformance: the backend verifies leanVM's claim-list statement
//! rather than the circuit EIP-8288 specifies, whose verification key the EIP still
//! lists as `TBD`.

use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSTARK, DependencyTriple, deduplicate_and_sort_dependencies,
};
use ethrex_common::{H256, U256};
use ethrex_dep_aggregation::leanvm::LeanVmAggregator;
use ethrex_dep_aggregation::test_support::leansphincs_dependency_from_seed as dependency;
use ethrex_dep_aggregation::{AggregateError, DependencyAggregator};

/// A real recursive aggregate, produced and then verified against the dependency
/// set a block would declare.
#[test]
#[ignore = "leanVM proving peaks at 9-11 GiB and takes ~1s"]
fn a_real_aggregate_verifies_against_the_dependencies_it_proves() {
    let agg = LeanVmAggregator::new();

    let (t1, w1) = dependency(1, [0x11; 32]);
    let (t2, w2) = dependency(2, [0x22; 32]);
    let expected = deduplicate_and_sort_dependencies(vec![t1, t2]);

    let proof = agg
        .aggregate(&[w1, w2], &[], None)
        .expect("aggregating two leanSPHINCS dependencies must succeed");

    agg.verify(&proof, &expected)
        .expect("the aggregate must discharge exactly the dependencies it proved");

    // The check EIP-8288's rule 2 omits. Without it any valid aggregate would
    // satisfy any block, since the proof does not bind the block's digest.
    let (other, _) = dependency(3, [0x33; 32]);
    assert!(
        agg.verify(&proof, &[other]).is_err(),
        "a proof must not satisfy a dependency set it does not cover"
    );
    assert!(
        agg.verify(&proof, &expected[..1]).is_err(),
        "nor a strict subset of what it proved"
    );
}

/// A builder's set moves between rounds -- a transaction is included, dropped or
/// expires -- so it must be able to publish a proof over less than its inputs
/// establish. Without that, an absorbed proof covering `{A, B}` makes a block
/// containing only `A` unbuildable.
#[test]
#[ignore = "leanVM proving peaks at 9-11 GiB and takes ~1s"]
fn an_aggregate_can_declare_a_subset_of_what_it_proves() {
    let agg = LeanVmAggregator::new();

    let (t1, w1) = dependency(5, [0x55; 32]);
    let (t2, w2) = dependency(6, [0x66; 32]);

    let narrowed = agg
        .aggregate(&[w1, w2], &[], Some(&[t1]))
        .expect("declaring a subset of the witnesses must succeed");

    agg.verify(&narrowed, &[t1])
        .expect("the narrowed proof discharges exactly what it declared");
    assert!(
        agg.verify(&narrowed, &deduplicate_and_sort_dependencies(vec![t1, t2]))
            .is_err(),
        "and does not discharge the dependency it discarded"
    );
}

/// Narrowing must fail loudly rather than quietly produce a proof covering less
/// than asked: a builder that got one would publish a block failing its own rule 2.
#[test]
fn declaring_a_dependency_nothing_establishes_is_refused() {
    let agg = LeanVmAggregator;
    let (_, w) = dependency(7, [0x77; 32]);
    let (absent, _) = dependency(8, [0x88; 32]);

    assert!(
        matches!(
            agg.aggregate(&[w], &[], Some(&[absent])),
            Err(AggregateError::ClaimsMismatch(_))
        ),
        "no witness and no child establishes that dependency"
    );
}

/// leanSTARK has no counterpart in leanVM, and the backend says so by name rather
/// than failing with something that reads like a corrupt proof.
#[test]
fn a_leanstark_dependency_is_reported_as_unsupported() {
    let triple = DependencyTriple {
        scheme: DEPENDENCY_SCHEME_LEANSTARK,
        data_hash: H256::zero(),
        verification_key_hash: H256::zero(),
    };
    // The scheme check runs before any proving or verifying, so this needs no
    // warm-up and no circuit.
    let agg = LeanVmAggregator;
    assert_eq!(
        agg.verify(&[], &[triple]),
        Err(AggregateError::SchemeUnsupported {
            scheme: DEPENDENCY_SCHEME_LEANSTARK
        })
    );
}

/// Both block-validity rules against one block carrying a real aggregate.
///
/// Rule 1 is the header's digest against the block's own transactions; rule 2 is the
/// proof against that same set. Together they are what makes a dependency binding:
/// rule 1 alone lets a block declare anything and prove nothing, rule 2 alone lets
/// the header disagree with the body.
#[test]
#[ignore = "leanVM proving peaks at 9-11 GiB and takes ~1s"]
fn a_block_carrying_a_real_aggregate_satisfies_both_rules() {
    use ethrex_common::types::{
        BlockBody, Frame, FrameMode, FrameTransaction, LEANSPHINCS_VERIFICATION_GAS,
        RecursiveStark, Transaction,
    };

    let agg = LeanVmAggregator::new();
    let (t1, w1) = dependency(10, [0xA1; 32]);
    let (t2, w2) = dependency(11, [0xB2; 32]);

    // A transaction declaring both dependencies in one frame.
    let mut data = t1.encode().to_vec();
    data.extend_from_slice(&t2.encode());
    let tx = FrameTransaction {
        frames: vec![Frame {
            mode: FrameMode::DepVerify as u8,
            flags: 0,
            target: None,
            gas_limit: 2 * LEANSPHINCS_VERIFICATION_GAS,
            state_gas_limit: 0,
            value: U256::zero(),
            data: data.into(),
        }],
        ..Default::default()
    };
    let body = BlockBody {
        transactions: vec![Transaction::FrameTransaction(tx)],
        ommers: Vec::new(),
        withdrawals: None,
    };

    let declared = body.dependencies();
    assert_eq!(declared.len(), 2, "the frame declares both dependencies");

    let proof = agg
        .aggregate(&[w1, w2], &[], Some(&declared))
        .expect("the builder aggregates exactly what the block declares");

    let header_entry = RecursiveStark {
        proof: proof.into(),
        block_deps_hash: body.block_deps_hash(),
    };

    // Rule 1.
    assert_eq!(header_entry.block_deps_hash, body.block_deps_hash());
    // Rule 2.
    agg.verify(&header_entry.proof, &declared)
        .expect("the aggregate discharges the block's dependencies");

    // A header that declares a different digest fails rule 1 while the proof still
    // verifies -- which is why rule 1 is not redundant.
    let tampered = RecursiveStark {
        proof: header_entry.proof.clone(),
        block_deps_hash: H256::from_low_u64_be(0xBAD),
    };
    assert_ne!(tampered.block_deps_hash, body.block_deps_hash());
    assert!(agg.verify(&tampered.proof, &declared).is_ok());
}

/// A witness filed under the wrong dependency must be refused, or an aggregate
/// could prove a claim the block never declared.
#[test]
fn a_witness_must_match_the_dependency_it_is_filed_under() {
    let (_, w) = dependency(4, [0x44; 32]);
    let mut tampered = w.clone();
    tampered.triple.verification_key_hash = H256::from_low_u64_be(0xBAD);

    let agg = LeanVmAggregator;
    assert!(matches!(
        agg.verify_witness(&tampered),
        Err(AggregateError::ClaimsMismatch(_))
    ));
    agg.verify_witness(&w)
        .expect("and the untampered one still checks out");
}

/// Measure what aggregation actually costs, which EIP-8288's gas constants assert
/// without data: it prices `verification_gas` as "the _expected_ cost of verifying
/// the recursive STARK" and gives 3,000 and 30,000 with no measurement behind them.
///
/// Reports proving wall time, verification wall time and proof size against the
/// dependency count. Peak memory is not measured in-process; run the whole binary
/// under a resident-set reporter for that. Ignored by default, and driven one count
/// per invocation so an external reporter attributes memory to a single aggregate:
///
/// `EIP8288_BENCH_DEPS=16 cargo test -p ethrex-test --features leanvm -- --ignored --nocapture bench_aggregation_cost`
#[test]
#[ignore = "benchmark; set EIP8288_BENCH_DEPS and run deliberately"]
fn bench_aggregation_cost() {
    use std::time::Instant;

    let n: usize = std::env::var("EIP8288_BENCH_DEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    assert!(n >= 1 && n <= 256, "1..=256 dependencies");

    let built = Instant::now();
    let agg = LeanVmAggregator::new();
    let warm_up = built.elapsed();

    let pairs: Vec<_> = (0..n)
        .map(|i| dependency(i as u8, [(i as u8).wrapping_mul(7); 32]))
        .collect();
    let triples: Vec<_> = pairs.iter().map(|(t, _)| *t).collect();
    let witnesses: Vec<_> = pairs.iter().map(|(_, w)| w.clone()).collect();
    let expected = deduplicate_and_sort_dependencies(triples);
    assert_eq!(
        expected.len(),
        n,
        "seeds must produce distinct dependencies"
    );

    let t = Instant::now();
    let proof = agg
        .aggregate(&witnesses, &[], Some(&expected))
        .expect("aggregation must succeed");
    let prove = t.elapsed();

    let t = Instant::now();
    agg.verify(&proof, &expected).expect("must verify");
    let verify = t.elapsed();

    println!(
        "EIP8288_BENCH deps={n} warm_up_ms={} prove_ms={} verify_ms={} proof_bytes={}",
        warm_up.as_millis(),
        prove.as_millis(),
        verify.as_millis(),
        proof.len()
    );
}

/// The mempool loop does not re-prove from witnesses. It absorbs the previous
/// round's aggregate and its peers' aggregates as children. Whether recursion is
/// cheaper than re-proving is what decides whether the loop is viable at
/// `AGGREGATION_INTERVAL`, and EIP-8288 assumes it is without measuring.
#[test]
#[ignore = "benchmark; set EIP8288_BENCH_DEPS and run deliberately"]
fn bench_recursive_absorption() {
    use std::time::Instant;

    let n: usize = std::env::var("EIP8288_BENCH_DEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);

    let agg = LeanVmAggregator::new();
    let pairs: Vec<_> = (0..n)
        .map(|i| dependency(i as u8, [(i as u8).wrapping_mul(7); 32]))
        .collect();
    let triples: Vec<_> = pairs.iter().map(|(t, _)| *t).collect();
    let witnesses: Vec<_> = pairs.iter().map(|(_, w)| w.clone()).collect();
    let expected = deduplicate_and_sort_dependencies(triples);

    // Round one: prove the set from raw witnesses.
    let t = Instant::now();
    let child = agg
        .aggregate(&witnesses, &[], Some(&expected))
        .expect("round one");
    let flat = t.elapsed();

    // Round two: absorb that aggregate as a child and add one new dependency, which
    // is what a mempool node does every AGGREGATION_INTERVAL.
    let (extra_t, extra_w) = dependency(250, [0xEE; 32]);
    let mut union = expected.clone();
    union.push(extra_t);
    let union = deduplicate_and_sort_dependencies(union);

    let t = Instant::now();
    let merged = agg
        .aggregate(&[extra_w], &[child.as_slice()], Some(&union))
        .expect("round two absorbs the child");
    let recursive = t.elapsed();

    agg.verify(&merged, &union).expect("merged must verify");

    println!(
        "EIP8288_RECURSE deps={n} flat_prove_ms={} absorb_child_plus_one_ms={} merged_bytes={}",
        flat.as_millis(),
        recursive.as_millis(),
        merged.len()
    );
}
