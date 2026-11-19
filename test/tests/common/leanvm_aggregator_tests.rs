//! The leanVM aggregation backend, exercised for real.
//!
//! Only compiled with the `leanvm` feature. A one-signature aggregate takes well
//! under a second and about 500 MiB, so the proving tests run by default; only the
//! benchmarks are `#[ignore]`d.
//!
//! What these establish is that the cryptography works -- signatures are real,
//! aggregates are real, and a proof over the wrong set fails -- under the guest
//! whose key is the restarted Daisugi network's `AGGREGATED_VK`.

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

    // The proof attests to one digest, so it satisfies no other set: neither a
    // different one nor a strict subset of what it proved.
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

/// leanSTARK is a scheme the backend verifies: a block declaring one is judged on
/// its proof, so declaring one with an empty proof is a malformed proof rather than
/// a dependency this node cannot check.
#[test]
fn a_leanstark_dependency_is_verified_not_refused() {
    let triple = DependencyTriple {
        scheme: DEPENDENCY_SCHEME_LEANSTARK,
        data_hash: H256::zero(),
        verification_key_hash: H256::zero(),
    };
    let agg = LeanVmAggregator;
    assert!(agg.supports_scheme(DEPENDENCY_SCHEME_LEANSTARK));
    assert_eq!(
        agg.verify(&[], &[triple]),
        Err(AggregateError::ProofMalformed)
    );
}

/// Both block-validity rules against one block carrying a real aggregate.
///
/// Rule 1 is the header's digest against the block's own transactions; rule 2 is the
/// proof against that same set. Together they are what makes a dependency binding:
/// rule 1 alone lets a block declare anything and prove nothing, rule 2 alone lets
/// the header disagree with the body.
#[test]
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
        stark_proof: None,
    };

    let declared = body.dependencies();
    assert_eq!(declared.len(), 2, "the frame declares both dependencies");

    let proof = agg
        .aggregate(&[w1, w2], &[], Some(&declared))
        .expect("the builder aggregates exactly what the block declares");

    let header_entry = RecursiveStark::commit(&proof, body.block_deps_hash());

    // Rule 1.
    assert_eq!(header_entry.block_deps_hash, body.block_deps_hash());
    // The header binds the proof the body carries.
    assert_eq!(
        header_entry.stark_proof_hash,
        ethrex_common::utils::keccak(&proof)
    );
    // Rule 2.
    agg.verify(&proof, &declared)
        .expect("the aggregate discharges the block's dependencies");

    // A header that declares a different digest fails rule 1 while the proof still
    // verifies -- which is why rule 1 is not redundant.
    let tampered = RecursiveStark {
        block_deps_hash: H256::from_low_u64_be(0xBAD),
        ..header_entry
    };
    assert_ne!(tampered.block_deps_hash, body.block_deps_hash());
    assert!(agg.verify(&proof, &declared).is_ok());
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

// ==================== Vectors from the network's own native package ====================

/// A vector from `fixtures/blockchain/daisugi-devnet-0/lean-vectors`, which the
/// native proving package of the network's reference client ships for its own
/// tests: real witnesses made at the revision the network pins.
fn network_vector(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/blockchain/daisugi-devnet-0/lean-vectors")
        .join(format!("{name}.hex"));
    let text = std::fs::read_to_string(&path).expect("vector exists");
    hex::decode(text.trim()).expect("vector is hex")
}

/// The dependency and witness of one network vector set, `sphincs` or `stark`.
fn network_witness(scheme: &str) -> ethrex_dep_aggregation::DependencyWitness {
    use ethrex_common::types::DEPENDENCY_SCHEME_LEANSPHINCS;
    let triple = DependencyTriple {
        scheme: if scheme == "sphincs" {
            DEPENDENCY_SCHEME_LEANSPHINCS
        } else {
            DEPENDENCY_SCHEME_LEANSTARK
        },
        data_hash: H256::from_slice(&network_vector(&format!("{scheme}-message"))),
        verification_key_hash: H256::from_slice(&network_vector(&format!("{scheme}-key"))),
    };
    let witness = network_vector(if scheme == "sphincs" {
        "sphincs-signature"
    } else {
        "stark-proof"
    });
    ethrex_dep_aggregation::DependencyWitness { triple, witness }
}

/// The package's aggregation key is the one this backend compiles.
#[test]
fn the_network_package_has_this_backends_key() {
    assert_eq!(
        network_vector("aggregated-key"),
        ethrex_dep_aggregation::leanvm::AGGREGATED_VK.to_vec()
    );
    assert_eq!(
        LeanVmAggregator.aggregated_vk().as_bytes(),
        &ethrex_dep_aggregation::leanvm::AGGREGATED_VK
    );
}

/// A leanSPHINCS signature and a leanSTARK proof made by the network's own
/// library verify here, and stop verifying when the message, the key or the
/// witness changes.
#[test]
fn the_network_packages_witnesses_verify() {
    let agg = LeanVmAggregator;
    for scheme in ["sphincs", "stark"] {
        let witness = network_witness(scheme);
        agg.verify_witness(&witness)
            .unwrap_or_else(|e| panic!("the {scheme} witness must verify: {e:?}"));

        let mut wrong_message = witness.clone();
        wrong_message.triple.data_hash = H256::zero();
        assert!(
            agg.verify_witness(&wrong_message).is_err(),
            "{scheme}: message"
        );

        let mut wrong_key = witness.clone();
        wrong_key.triple.verification_key_hash = H256::repeat_byte(0x11);
        assert!(agg.verify_witness(&wrong_key).is_err(), "{scheme}: key");

        let mut flipped = witness.clone();
        let last = flipped.witness.len() - 1;
        flipped.witness[last] ^= 1;
        assert!(
            agg.verify_witness(&flipped).is_err(),
            "{scheme}: witness byte"
        );
    }
}

/// A block proof over one leanSPHINCS and one leanSTARK dependency, built here
/// from the network library's witnesses, verifies against the block's dependency
/// set under the network's key, and not against a different set.
#[test]
fn a_block_proof_over_the_network_packages_witnesses_verifies() {
    let agg = LeanVmAggregator::new();
    let sphincs = network_witness("sphincs");
    let stark = network_witness("stark");
    let expected = deduplicate_and_sort_dependencies(vec![sphincs.triple, stark.triple]);

    let proof = agg
        .aggregate(&[sphincs.clone(), stark], &[], None)
        .expect("aggregating the network's witnesses must succeed");
    agg.verify(&proof, &expected)
        .expect("the proof must discharge the block's dependency set");
    assert!(
        agg.verify(&proof, &[sphincs.triple]).is_err(),
        "nor discharge a set it does not prove"
    );
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
    assert!((1..=256).contains(&n), "1..=256 dependencies");

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
    let mut union = expected;
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

/// The proof of daisugi-devnet-0's block 673, as the network's reference client
/// produced it, verifies here against that block's dependency set, and a single
/// flipped byte makes it fail. The block's one frame transaction declares one
/// leanSTARK dependency with a zero `data_hash` (its own signature hash).
#[test]
fn the_devnet_block_673_proof_verifies_and_a_flipped_byte_does_not() {
    use ethrex_common::types::{Transaction, dependencies_hash};
    use std::str::FromStr;

    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/blockchain/daisugi-devnet-0/block-673-stark-proof.hex");
    let proof = hex::decode(std::fs::read_to_string(path).unwrap().trim()).unwrap();
    assert_eq!(
        ethrex_common::utils::keccak(&proof),
        H256::from_str("0x455907bb1fc322d21d0e5a5ed9b09be3f78be381fb134a2af30fe5a7512d817e")
            .unwrap(),
        "the fixture is the proof block 673's header commits to"
    );

    // Block 673's only transaction, as rebuilt from the network's RPC.
    let raw = "06f901238429d103d8c0809405a20b6083bd71c58c10cbfa9db8036bf5a8e206f898cb010380c5830186a0808080f86b048080c48275308080b860000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000004f0922a927365fec2cf13867145a3de76b3d26385681cd100c9b3d23c3c03d46de028094000000000000000000000000000000000000deadc482c350807b80f85cf85a019405a20b6083bd71c58c10cbfa9db8036bf5a8e20680b841016f7832e83fd7c388595d7b446ab9d1b05922bdd84cbc190e73a298f540e3d7a03470c708c423657fae82b574f91da2d15c0565c8aa564ea924e19b969145a544cd873ff2e795f500008477359400c0";
    let Transaction::FrameTransaction(tx) =
        Transaction::decode_canonical(&hex::decode(raw).unwrap()).unwrap()
    else {
        panic!("block 673's transaction is a frame transaction");
    };
    let dependencies = tx.dependencies();
    assert_eq!(
        dependencies_hash(&dependencies),
        H256::from_str("0xe79b80cdcc7c68c07bfa15f80eef9fe10a92b5adc0246b5074692d88e4543f72")
            .unwrap()
    );

    let agg = LeanVmAggregator::new();
    agg.verify(&proof, &dependencies)
        .expect("the network's proof verifies under its AGGREGATED_VK");

    let mut flipped = proof;
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0x01;
    assert!(agg.verify(&flipped, &dependencies).is_err());
}
