//! Experiments behind the "maximum statement size" framing of EIP-8288's circuit
//! requirements. Scratch benchmarks, not part of any PR.
//!
//! Two questions, each answered with real leanVM proofs:
//!
//! - `bench_round_vs_children`: what one mempool aggregation round costs as a
//!   function of how many child aggregates it absorbs (one per peer, plus the node's
//!   own previous aggregate), and what happens past leanVM's `MAX_RECURSIONS`.
//! - `bench_statement_size`: how proof size, verification and the next round's
//!   absorption scale with the number of claims an aggregate covers.
//!
//! Driven by environment variables, one configuration per process so an external
//! resident-set reporter attributes memory to a single run.

use std::time::{Duration, Instant};

use ethrex_common::types::{DependencyTriple, deduplicate_and_sort_dependencies};
use ethrex_dep_aggregation::DependencyAggregator;
use ethrex_dep_aggregation::DependencyWitness;
use ethrex_dep_aggregation::leanvm::LeanVmAggregator;
use ethrex_dep_aggregation::test_support::leansphincs_dependency_from_seed as dependency;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Claim `i`: a distinct `(key, message)` pair. Keys repeat every 251 claims and the
/// message carries `i`, so every index yields a different claim.
fn claim(i: usize) -> (DependencyTriple, DependencyWitness) {
    let mut message = [0xA5u8; 32];
    message[..8].copy_from_slice(&(i as u64).to_be_bytes());
    dependency((i % 251) as u8, message)
}

fn claims(range: std::ops::Range<usize>) -> (Vec<DependencyTriple>, Vec<DependencyWitness>) {
    range.map(claim).unzip()
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

/// Gas a block pays per declared leanSPHINCS claim, from ethrex's own frame-transaction
/// gas functions (calldata cost, calldata floor, per-frame cost, frame budget) plus
/// EIP-8288's `recursive_stark_gas`, which ethrex does not charge yet:
/// `LEANSTARK_VERIFICATION_GAS` per declared triple, added to intrinsic gas.
///
/// The marginal is taken over one dependency frame holding 256 triples, the most a
/// frame may declare, so per-frame and per-transaction overhead is amortized the way
/// a block stuffed with claims would amortize it.
#[test]
#[ignore = "experiment"]
fn gas_per_declared_claim() {
    use bytes::Bytes;
    use ethrex_common::types::{
        DEPENDENCY_SCHEME_LEANSPHINCS, Frame, FrameTransaction, LEANSPHINCS_VERIFICATION_GAS,
        LEANSTARK_VERIFICATION_GAS,
    };
    use ethrex_common::{Address, H256, U256};
    use once_cell::sync::OnceCell;

    // Hashes are uniformly random bytes, so about 1 in 256 is zero, as for real
    // message and key bytes.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut random_h256 = || {
        let mut out = [0u8; 32];
        for chunk in out.chunks_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            chunk.copy_from_slice(&state.to_le_bytes());
        }
        H256(out)
    };

    const TRIPLES: u64 = 256;
    let mut data = Vec::with_capacity(96 * TRIPLES as usize);
    for _ in 0..TRIPLES {
        let triple = DependencyTriple {
            scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
            data_hash: random_h256(),
            verification_key_hash: random_h256(),
        };
        data.extend_from_slice(&triple.encode());
    }
    let dep_frame = Frame {
        mode: 3,
        flags: 0,
        target: None,
        gas_limit: LEANSPHINCS_VERIFICATION_GAS * TRIPLES,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(data),
    };
    let tx = |frames: Vec<Frame>| FrameTransaction {
        chain_id: 1,
        nonce: 0,
        sender: Address::from_low_u64_be(0xABCD),
        frames,
        signatures: vec![],
        max_priority_fee_per_gas: U256::from(1u64),
        max_fee_per_gas: U256::from(1u64),
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        inner_hash: OnceCell::new(),
        cached_canonical: OnceCell::new(),
    };
    let base = tx(vec![]);
    let with = tx(vec![dep_frame]);

    let per = |a: u64, b: u64| (a - b) as f64 / TRIPLES as f64;
    let standard = per(with.standard_gas_limit(), base.standard_gas_limit());
    let floor = per(with.calldata_floor_total(), base.calldata_floor_total());
    let ethrex_max = per(with.max_gas(), base.max_gas());
    let total = ethrex_max + LEANSTARK_VERIFICATION_GAS as f64;
    println!(
        "EIP8288_GAS per_claim standard={standard:.0} floor={floor:.0} ethrex_max={ethrex_max:.0} recursive_stark_gas={} total={total:.0}",
        LEANSTARK_VERIFICATION_GAS
    );
    for gas_limit in [30_000_000u64, 45_000_000, 60_000_000, 100_000_000, 200_000_000] {
        println!(
            "EIP8288_GAS block_gas_limit={gas_limit} max_claims_per_block={}",
            (gas_limit as f64 / total).floor()
        );
    }
}

/// One aggregation round absorbing `EIP8288_CHILDREN` child aggregates, each covering
/// `EIP8288_CLAIMS_PER_CHILD` claims, plus one new raw claim.
#[test]
#[ignore = "experiment; set EIP8288_CHILDREN and run deliberately"]
fn bench_round_vs_children() {
    let children = env_usize("EIP8288_CHILDREN", 4);
    let per_child = env_usize("EIP8288_CLAIMS_PER_CHILD", 8);
    let agg = LeanVmAggregator::new();

    let t = Instant::now();
    let mut child_proofs = Vec::with_capacity(children);
    let mut union = Vec::new();
    for c in 0..children {
        let (triples, witnesses) = claims(c * per_child..(c + 1) * per_child);
        let declared = deduplicate_and_sort_dependencies(triples);
        child_proofs.push(
            agg.aggregate(&witnesses, &[], Some(&declared))
                .expect("child aggregate"),
        );
        union.extend(declared);
    }
    let build = t.elapsed();

    let (new_t, new_w) = claim(1_000_000);
    union.push(new_t);
    let union = deduplicate_and_sort_dependencies(union);
    let refs: Vec<&[u8]> = child_proofs.iter().map(Vec::as_slice).collect();

    let t = Instant::now();
    let round = agg.aggregate(&[new_w], &refs, Some(&union));
    let round_time = t.elapsed();

    match round {
        Ok(proof) => {
            let t = Instant::now();
            agg.verify(&proof, &union).expect("round aggregate verifies");
            let verify = t.elapsed();
            println!(
                "EIP8288_ROUND children={children} claims_per_child={per_child} statement={} build_children_ms={} round_ms={} verify_ms={} proof_bytes={}",
                union.len(),
                build.as_millis(),
                round_time.as_millis(),
                verify.as_millis(),
                proof.len()
            );
        }
        Err(e) => println!(
            "EIP8288_ROUND children={children} claims_per_child={per_child} statement={} REFUSED after {} ms: {e:?}",
            union.len(),
            round_time.as_millis()
        ),
    }
}

/// An aggregate covering `EIP8288_CLAIMS` claims, built as a tree of at most 16
/// children per node over leaves of 16 claims. Reports its size, its verification
/// time, and the cost of the next round absorbing it plus one new claim.
#[test]
#[ignore = "experiment; set EIP8288_CLAIMS and run deliberately"]
fn bench_statement_size() {
    const LEAF: usize = 16;
    const FAN_IN: usize = 16;
    let n = env_usize("EIP8288_CLAIMS", 64);
    assert!(n >= LEAF && n.is_multiple_of(LEAF), "a multiple of {LEAF} claims");
    let agg = LeanVmAggregator::new();

    let t = Instant::now();
    let mut level: Vec<(Vec<u8>, Vec<DependencyTriple>)> = (0..n / LEAF)
        .map(|leaf| {
            let (triples, witnesses) = claims(leaf * LEAF..(leaf + 1) * LEAF);
            let declared = deduplicate_and_sort_dependencies(triples);
            let proof = agg
                .aggregate(&witnesses, &[], Some(&declared))
                .expect("leaf aggregate");
            (proof, declared)
        })
        .collect();
    while level.len() > 1 {
        level = level
            .chunks(FAN_IN)
            .map(|group| {
                let refs: Vec<&[u8]> = group.iter().map(|(p, _)| p.as_slice()).collect();
                let union = deduplicate_and_sort_dependencies(
                    group.iter().flat_map(|(_, d)| d.iter().copied()).collect(),
                );
                let proof = agg
                    .aggregate(&[], &refs, Some(&union))
                    .expect("merge aggregate");
                (proof, union)
            })
            .collect();
    }
    let build = t.elapsed();
    let (proof, declared) = level.pop().expect("one root aggregate");
    assert_eq!(declared.len(), n);

    let verify = median(
        (0..5)
            .map(|_| {
                let t = Instant::now();
                agg.verify(&proof, &declared).expect("root verifies");
                t.elapsed()
            })
            .collect(),
    );

    let (new_t, new_w) = claim(1_000_000);
    let mut union = declared;
    union.push(new_t);
    let union = deduplicate_and_sort_dependencies(union);
    let t = Instant::now();
    let next = agg
        .aggregate(&[new_w], &[proof.as_slice()], Some(&union))
        .expect("next round absorbs the root");
    let absorb = t.elapsed();

    println!(
        "EIP8288_STATEMENT claims={n} build_ms={} proof_bytes={} verify_ms={} next_round_absorb_ms={} next_proof_bytes={}",
        build.as_millis(),
        proof.len(),
        verify.as_millis(),
        absorb.as_millis(),
        next.len()
    );
}
